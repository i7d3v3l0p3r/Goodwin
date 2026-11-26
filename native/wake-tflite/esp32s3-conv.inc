// Included inside LiveKit's lkww_conv namespace. Grouped convolutions keep
// their original kernel. For dense int8 convolutions, use the ESP32-S3 PIE
// dot product and fold the input zero point into a per-channel correction.
extern "C" bool goodwin_tflite_use_esp32s3_simd();
extern "C" int32_t esp_nn_dot_s8_unaligned_esp32s3(const int8_t* input,
                                                  const int8_t* weights,
                                                  int32_t vectors);
extern "C" int32_t esp_nn_dot_s8_aligned_esp32s3(const int8_t* input,
                                                const int8_t* weights,
                                                int32_t bytes);

struct PackedWeights {
  static constexpr size_t kCapacity = 64 * 1024;
  void* storage = nullptr;
  size_t used = 0;

  ~PackedWeights() { free(storage); }

  int8_t* Allocate(size_t bytes) {
    if (bytes > kCapacity - used) return nullptr;
    if (storage == nullptr && posix_memalign(&storage, 16, kCapacity) != 0) return nullptr;
    auto* output = static_cast<int8_t*>(storage) + used;
    used += bytes;
    return output;
  }
};

inline const int32_t* PrepareSimd(TfLiteContext* context, const TfLiteTensor& input,
                                  const TfLiteTensor& weights, const TfLiteTensor& output,
                                  const TfLiteTensor* bias, const int8_t** packed) {
  *packed = nullptr;
  if (!goodwin_tflite_use_esp32s3_simd() || input.type != kTfLiteInt8 ||
      weights.type != kTfLiteInt8 || output.type != kTfLiteInt8 ||
      weights.allocation_type != kTfLiteMmapRo ||
      (bias != nullptr && bias->type != kTfLiteInt32) ||
      input.dims->data[3] != weights.dims->data[3]) return nullptr;
  const int channels = weights.dims->data[0];
  const int row = weights.dims->data[1] * weights.dims->data[2] * weights.dims->data[3];
  if (row < 16 || row > 2048) return nullptr;
  auto* correction = static_cast<int32_t*>(context->AllocatePersistentBuffer(
      context, channels * sizeof(int32_t)));
  if (correction == nullptr) return nullptr;
  const int8_t* weights_row = weights.data.int8;
  const size_t padded_row = (row + 15) & ~size_t{15};
  int8_t* packed_row = nullptr;
  // Reuse padded, aligned weights for convolutions that consume them repeatedly.
  if (output.dims->data[1] * output.dims->data[2] > kSmallPixels) {
    auto* pool = static_cast<PackedWeights*>(lkww_tfl::GetMicroContext(context)->external_context());
    packed_row = pool->Allocate(channels * padded_row);
    *packed = packed_row;
  }
  for (int channel = 0; channel < channels; ++channel, weights_row += row) {
    int32_t sum = 0;
    for (int i = 0; i < row; ++i) sum += weights_row[i];
    correction[channel] = -input.params.zero_point * sum;
    if (packed_row != nullptr) {
      memcpy(packed_row, weights_row, row);
      memset(packed_row + row, 0, padded_row - row);
      packed_row += padded_row;
    }
  }
  return correction;
}

inline void ConvSimd(const TfLiteConvParams& p, const lkww_tfl::OpDataConv& d,
                     const lkww_tfl::RuntimeShape& is, const int8_t* in,
                     const lkww_tfl::RuntimeShape& fs, const int8_t* filt, const int32_t* bias,
                     const lkww_tfl::RuntimeShape& os, int8_t* out,
                     const int32_t* correction, const int8_t* packed) {
  // Keep the repeatedly read input patch on the ESP-IDF task's internal stack.
  alignas(16) int8_t patch[2048];
  const int ih = is.Dims(1), iw = is.Dims(2), depth = is.Dims(3);
  const int fh = fs.Dims(1), fw = fs.Dims(2), row = fh * fw * depth;
  const int vectors = row / 16, tail = vectors * 16;
  const int padded_row = (row + 15) & ~15;
  memset(patch + row, 0, padded_row - row);
  const int oh = os.Dims(1), ow = os.Dims(2), channels = os.Dims(3);
  for (int batch = 0; batch < is.Dims(0); ++batch) {
    for (int oy = 0; oy < oh; ++oy) {
      const int y0 = oy * p.stride_height - d.padding.height;
      for (int ox = 0; ox < ow; ++ox) {
        const int x0 = ox * p.stride_width - d.padding.width;
        int8_t* next = patch;
        for (int ky = 0; ky < fh; ++ky) {
          const int y = y0 + ky * p.dilation_height_factor;
          for (int kx = 0; kx < fw; ++kx, next += depth) {
            const int x = x0 + kx * p.dilation_width_factor;
            if (y < 0 || y >= ih || x < 0 || x >= iw) {
              memset(next, d.input_zero_point, depth);
            } else {
              memcpy(next, in + ((batch * ih + y) * iw + x) * depth, depth);
            }
          }
        }
        const int8_t* w = packed != nullptr ? packed : filt;
        const int pitch = packed != nullptr ? padded_row : row;
        for (int channel = 0; channel < channels; ++channel, w += pitch) {
          int32_t acc;
          if (packed != nullptr) {
            acc = esp_nn_dot_s8_aligned_esp32s3(patch, w, padded_row);
          } else {
            acc = esp_nn_dot_s8_unaligned_esp32s3(patch, w, vectors);
            for (int i = tail; i < row; ++i) acc += static_cast<int32_t>(patch[i]) * w[i];
          }
          acc += correction[channel] + (bias != nullptr ? bias[channel] : 0);
          *out++ = Requantize(acc, d, channel, d.output_zero_point);
        }
      }
    }
  }
}

inline TfLiteStatus DepthwiseEval(TfLiteContext* context, TfLiteNode* node) {
  using namespace tflite::micro;
  const auto& p = *static_cast<const TfLiteDepthwiseConvParams*>(node->builtin_data);
  const auto* input = GetEvalInput(context, node, kDepthwiseConvInputTensor);
  const auto* filter = GetEvalInput(context, node, kDepthwiseConvWeightsTensor);
  if (p.depth_multiplier != 1 || input->type != kTfLiteInt8 || filter->type != kTfLiteInt8) {
    return Register_DEPTHWISE_CONV_2D().invoke(context, node);
  }
  const auto* bias = NumInputs(node) == 3 ? GetEvalInput(context, node, kDepthwiseConvBiasTensor) : nullptr;
  const auto* biases = GetOptionalTensorData<int32_t>(bias);
  auto* output = GetEvalOutput(context, node, kDepthwiseConvOutputTensor);
  const auto is = GetTensorShape(input), fs = GetTensorShape(filter), os = GetTensorShape(output);
  const auto* in = GetTensorData<int8_t>(input);
  const auto* w = GetTensorData<int8_t>(filter);
  auto* out = GetTensorData<int8_t>(output);
  const auto& d = *static_cast<const OpDataConv*>(node->user_data);
  const int ih = is.Dims(1), iw = is.Dims(2), channels = is.Dims(3);
  const int fh = fs.Dims(1), fw = fs.Dims(2), oh = os.Dims(1), ow = os.Dims(2);
  constexpr int kBlock = 16;
  for (int batch = 0; batch < is.Dims(0); ++batch) {
    for (int oy = 0; oy < oh; ++oy) {
      const int y0 = oy * p.stride_height - d.padding.height;
      for (int ox = 0; ox < ow; ++ox) {
        const int x0 = ox * p.stride_width - d.padding.width;
        for (int first = 0; first < channels; first += kBlock) {
          const int count = channels - first < kBlock ? channels - first : kBlock;
          int32_t acc[kBlock] = {};
          for (int ky = 0; ky < fh; ++ky) {
            const int y = y0 + ky * p.dilation_height_factor;
            if (y < 0 || y >= ih) continue;
            for (int kx = 0; kx < fw; ++kx) {
              const int x = x0 + kx * p.dilation_width_factor;
              if (x < 0 || x >= iw) continue;
              const auto* signal = in + ((batch * ih + y) * iw + x) * channels + first;
              const auto* tap = w + (ky * fw + kx) * channels + first;
              for (int i = 0; i < count; ++i) {
                acc[i] += (static_cast<int32_t>(signal[i]) - d.input_zero_point) * tap[i];
              }
            }
          }
          for (int i = 0; i < count; ++i) {
            const int ch = first + i;
            *out++ = Requantize(acc[i] + (biases != nullptr ? biases[ch] : 0), d, ch, d.output_zero_point);
          }
        }
      }
    }
  }
  return kTfLiteOk;
}

inline TFLMRegistration DepthwiseRegistration() {
  auto registration = tflite::micro::Register_DEPTHWISE_CONV_2D();
  if (goodwin_tflite_use_esp32s3_simd()) registration.invoke = DepthwiseEval;
  return registration;
}
