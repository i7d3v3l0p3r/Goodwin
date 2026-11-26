#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <memory>
#include <new>

#include "frontend_util.h"
#include "lkww_tflm.h"
#include "tensorflow/lite/micro/micro_resource_variable.h"

namespace {
constexpr unsigned kHop = 160;
constexpr unsigned kFeatures = 40;
constexpr unsigned kWindow = 8;
constexpr unsigned kCutoff = 249;
constexpr unsigned kCooldown = 75;

struct Detector {
    alignas(16) uint8_t variables[1536]{};
    alignas(16) uint8_t arena[32 * 1024]{};
    tflite::MicroMutableOpResolver<20> resolver;
    lkww_conv::PackedWeights weights;
    std::unique_ptr<tflite::MicroInterpreter> model;
    FrontendState frontend{};
    TfLiteTensor* input = nullptr;
    TfLiteTensor* output = nullptr;
    unsigned probabilities[kWindow]{};
    unsigned next = 0;
    unsigned quiet_slices = 0;
    bool ready = false;

    ~Detector() { FrontendFreeStateContents(&frontend); }

    bool open(const tflite::Model* flat_model) {
        if (resolver.AddCallOnce() != kTfLiteOk || resolver.AddVarHandle() != kTfLiteOk ||
            resolver.AddReshape() != kTfLiteOk || resolver.AddReadVariable() != kTfLiteOk ||
            resolver.AddStridedSlice() != kTfLiteOk || resolver.AddConcatenation() != kTfLiteOk ||
            resolver.AddAssignVariable() != kTfLiteOk ||
            resolver.AddConv2D(lkww_conv::Registration()) != kTfLiteOk ||
            resolver.AddMul() != kTfLiteOk || resolver.AddAdd() != kTfLiteOk ||
            resolver.AddMean() != kTfLiteOk || resolver.AddFullyConnected() != kTfLiteOk ||
            resolver.AddLogistic() != kTfLiteOk || resolver.AddQuantize() != kTfLiteOk ||
            resolver.AddDepthwiseConv2D(lkww_conv::DepthwiseRegistration()) != kTfLiteOk ||
            resolver.AddAveragePool2D() != kTfLiteOk || resolver.AddMaxPool2D() != kTfLiteOk ||
            resolver.AddPad() != kTfLiteOk || resolver.AddPack() != kTfLiteOk ||
            resolver.AddSplitV() != kTfLiteOk) return false;
        auto* memory = tflite::MicroAllocator::Create(variables, sizeof(variables));
        auto* resource = tflite::MicroResourceVariables::Create(memory, 20);
        if (resource == nullptr) return false;
        model.reset(new (std::nothrow) tflite::MicroInterpreter(flat_model, resolver, arena, sizeof(arena), resource));
        if (!model || model->SetMicroExternalContext(&weights) != kTfLiteOk ||
            model->AllocateTensors() != kTfLiteOk) return false;
        input = model->input(0);
        output = model->output(0);
        if (!input || !output || input->type != kTfLiteInt8 || input->bytes != 3 * kFeatures ||
            output->type != kTfLiteUInt8 || output->bytes != 1) return false;

        FrontendConfig config{};
        config.window.size_ms = 30;
        config.window.step_size_ms = 10;
        config.filterbank.num_channels = kFeatures;
        config.filterbank.lower_band_limit = 125.0f;
        config.filterbank.upper_band_limit = 7500.0f;
        config.noise_reduction.smoothing_bits = 10;
        config.noise_reduction.even_smoothing = 0.025f;
        config.noise_reduction.odd_smoothing = 0.06f;
        config.noise_reduction.min_signal_remaining = 0.05f;
        config.pcan_gain_control.enable_pcan = true;
        config.pcan_gain_control.strength = 0.95f;
        config.pcan_gain_control.offset = 80.0f;
        config.pcan_gain_control.gain_bits = 21;
        config.log_scale.enable_log = true;
        config.log_scale.scale_shift = 6;
        if (!FrontendPopulateState(&config, &frontend, 16000)) return false;
        reset();
        return ready;
    }

    void reset() {
        FrontendReset(&frontend);
        // Two silent hops fill the overlap before the first 30 ms audio block.
        static const int16_t history[kHop * 2] = {};
        size_t consumed = 0;
        const auto primed = FrontendProcessSamples(&frontend, history, kHop * 2, &consumed);
        std::fill_n(probabilities, kWindow, 0u);
        next = 0;
        quiet_slices = 0;
        ready = consumed == kHop * 2 && primed.size == 0 && model->Reset() == kTfLiteOk;
    }

    int raw_probability(const int16_t* pcm) {
        if (!ready) return -1;
        for (unsigned hop = 0; hop < 3; ++hop) {
            size_t consumed = 0;
            const auto mel = FrontendProcessSamples(&frontend, pcm + hop * kHop, kHop, &consumed);
            if (consumed != kHop || mel.size != kFeatures) return -1;
            for (unsigned i = 0; i < kFeatures; ++i) {
                const int quantized = (mel.values[i] * 256 + 333) / 666 - 128;
                input->data.int8[hop * kFeatures + i] = std::clamp(quantized, -128, 127);
            }
        }
        if (model->Invoke() != kTfLiteOk) return -1;
        return output->data.uint8[0];
    }

    float process(const int16_t* pcm) {
        const int probability = raw_probability(pcm);
        if (probability < 0) return -1.0f;
        probabilities[next] = probability;
        next = (next + 1) % kWindow;
        if (static_cast<unsigned>(probability) < kCutoff) quiet_slices = std::min(quiet_slices + 3, kCooldown);
        if (quiet_slices < kCooldown) return 0.0f;
        unsigned sum = 0;
        for (auto score : probabilities) sum += score;
        return static_cast<float>(sum) / (kWindow * 255);
    }
};
}

extern "C" {
void* goodwin_microwakeword_open(const uint8_t* bytes, size_t length) {
    if (!bytes || !length || reinterpret_cast<uintptr_t>(bytes) % 16 != 0) return nullptr;
    flatbuffers::Verifier checker(bytes, length);
    if (!tflite::VerifyModelBuffer(checker)) return nullptr;
    const auto* model = tflite::GetModel(bytes);
    if (model->version() != 3 || !model->subgraphs() || model->subgraphs()->size() == 0) return nullptr;
    auto detector = std::unique_ptr<Detector>(new (std::nothrow) Detector());
    return detector && detector->open(model) ? detector.release() : nullptr;
}

void goodwin_microwakeword_close(void* detector) { delete static_cast<Detector*>(detector); }
void goodwin_microwakeword_reset(void* detector) { static_cast<Detector*>(detector)->reset(); }
float goodwin_microwakeword_step(void* detector, const int16_t* pcm) {
    return static_cast<Detector*>(detector)->process(pcm);
}
}
