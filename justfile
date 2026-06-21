# Crossbook task runner. Run `just` with no args to list recipes.

default:
    @just --list

# Full gate: format check + clippy + rust tests + contract tests. M0 acceptance target.
check: fmt-check clippy test forge-test

fmt:
    cargo fmt --all
    cd contracts && forge fmt

fmt-check:
    cargo fmt --all -- --check
    cd contracts && forge fmt --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

forge-test:
    cd contracts && forge test

# Matching core microbenchmarks (M1 onward).
bench:
    cargo bench -p crossbook-core

# Local devnet: Postgres + Anvil.
dev:
    docker compose up -d

deploy-local:
    cd contracts && forge script script/Deploy.s.sol --rpc-url $RPC_URL --broadcast

# Full end to end flow against Anvil (runs at M5).
e2e:
    @echo "e2e: implemented at M5"
