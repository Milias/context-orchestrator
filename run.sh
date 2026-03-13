#!/usr/bin/env bash
export HTTPS_PROXY=socks5h://localhost:9050
export SSL_CERT_FILE="$(dirname "$0")/ca-bundle.crt"
export ANTHROPIC_BASE_URL=https://litellm-v2.trading.imc.intra/
export ANTHROPIC_AUTH_TOKEN=sk-12Zcd0gUrgzXaepi3KVdbQ
export ANTHROPIC_MODEL='claude-opus-4-6'

cargo run "$@"
