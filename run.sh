#!/usr/bin/env bash
export ANTHROPIC_BASE_URL=https://litellm-v2.trading.imc.intra/
export ANTHROPIC_AUTH_TOKEN=sk-12Zcd0gUrgzXaepi3KVdbQ
export ANTHROPIC_MODEL='claude-opus-4-6'

cargo run "$@"
