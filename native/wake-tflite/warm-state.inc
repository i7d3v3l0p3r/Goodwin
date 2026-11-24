  bool save_warm_state() {
    if (score_->bytes != 1 || !valid_scale(mel_) || !valid_scale(score_)) return false;
    for (int i = 0; i < n_states_; ++i) {
      if (!valid_scale(state_[i]) || !valid_scale(state_out_[i])) return false;
    }
    lkww_frontend_reset(&fe_);
    for (int i = 0; i < n_states_; ++i) ZeroState(state_[i]);
    static const int16_t silence[LKWW_CHUNK] = {};
    for (int i = 0; i < kWarmupSteps; ++i) {
      const float score = Process(silence);
      if (!std::isfinite(score) || score < 0.0f || score > 1.0f) return false;
    }
    size_t bytes = 0;
    for (int i = 0; i < n_states_; ++i) bytes += state_[i]->bytes;
    warm_state_ = static_cast<uint8_t*>(malloc(bytes ? bytes : 1));
    if (!warm_state_) return false;
    uint8_t* next = warm_state_;
    for (int i = 0; i < n_states_; ++i) {
      memcpy(next, state_[i]->data.raw, state_[i]->bytes);
      next += state_[i]->bytes;
    }
    return true;
  }

  void restore_warm_state() {
    lkww_frontend_reset(&fe_);
    const uint8_t* next = warm_state_;
    for (int i = 0; i < n_states_; ++i) {
      memcpy(state_[i]->data.raw, next, state_[i]->bytes);
      next += state_[i]->bytes;
    }
  }

  static bool valid_scale(const TfLiteTensor* tensor) {
    return std::isfinite(tensor->params.scale) && tensor->params.scale > 0 &&
           -128 <= tensor->params.zero_point && tensor->params.zero_point <= 127;
  }
