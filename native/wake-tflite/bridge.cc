#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <new>

#include "lkww_tflm.h"

namespace {

constexpr size_t kArenaBytes = 160 * 1024;

struct Detector {
    alignas(16) uint8_t arena[kArenaBytes];
    LkwwDetector model;

    explicit Detector(const uint8_t* bytes) : model(bytes, arena, sizeof(arena), false) {}
};

bool valid_signature(const tflite::Model* model) {
    const auto* graphs = model->subgraphs();
    const auto* signatures = model->signature_defs();
    if (model->version() != 3 || !graphs || graphs->size() != 1 ||
        !signatures || signatures->size() != 1 || !model->buffers() ||
        !model->operator_codes()) return false;

    const auto* graph = graphs->Get(0);
    const auto* signature = signatures->Get(0);
    const auto* inputs = signature->inputs();
    const auto* outputs = signature->outputs();
    if (signature->subgraph_index() != 0 || !graph->tensors() || !graph->operators() ||
        !graph->inputs() || !graph->outputs() || !inputs || !outputs ||
        inputs->size() != graph->inputs()->size() || outputs->size() != graph->outputs()->size() ||
        inputs->size() != outputs->size() || inputs->size() == 0 ||
        inputs->size() > LkwwDetector::kMaxStates + 1) return false;

    for (const auto* map : {inputs, outputs}) {
        for (const auto* tensor : *map) {
            if (!tensor->name() || tensor->tensor_index() >= graph->tensors()->size()) return false;
        }
    }
    if (const auto* metadata = model->metadata()) {
        for (const auto* entry : *metadata) {
            if (!entry->name() || entry->name()->string_view() != "livekit_wakeword.mel_params") continue;
            if (entry->buffer() >= model->buffers()->size()) return false;
            const auto* values = model->buffers()->Get(entry->buffer())->data();
            if (!values || values->size() != 3 * sizeof(float)) return false;
            float params[3];
            std::memcpy(params, values->data(), sizeof(params));
            if (!std::isfinite(params[0]) || params[0] <= 0 || !std::isfinite(params[1]) ||
                !std::isfinite(params[2]) || params[2] <= 0) return false;
        }
    }
    return true;
}

}

extern "C" {

void* goodwin_tflite_open(const uint8_t* bytes, size_t len) {
    flatbuffers::Verifier verifier(bytes, len);
    if (!tflite::VerifyModelBuffer(verifier)) return nullptr;
    if (!valid_signature(tflite::GetModel(bytes))) return nullptr;
    auto* storage = std::malloc(sizeof(Detector));
    if (!storage) return nullptr;
    auto* detector = new (storage) Detector(bytes);
    if (detector->model.ok() && detector->model.save_warm_state()) return detector;
    detector->~Detector();
    std::free(storage);
    return nullptr;
}

void goodwin_tflite_close(void* handle) {
    static_cast<Detector*>(handle)->~Detector();
    std::free(handle);
}

void goodwin_tflite_reset(void* handle) {
    static_cast<Detector*>(handle)->model.restore_warm_state();
}

float goodwin_tflite_step(void* handle, const int16_t* pcm) {
    return static_cast<Detector*>(handle)->model.Process(pcm);
}

}
