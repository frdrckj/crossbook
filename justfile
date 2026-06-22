# Crossbook task runner. Run `just` with no args to list recipes.

default:
    @just --list

# Full gate: format check + clippy + rust tests + contract tests. M0 acceptance target.
check: fmt-check clippy test forge-test

fmt:
    cargo fmt --all
    taplo fmt
    cd contracts && forge fmt

fmt-check:
    cargo fmt --all -- --check
    taplo fmt --check
    cd contracts && forge fmt --check

# Format the Cargo and other TOML manifests with taplo.
fmt-toml:
    taplo fmt

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo nextest run --workspace
    cargo test --workspace --doc

forge-test:
    cd contracts && forge test

# Matching core microbenchmarks (M1 onward).
bench:
    cargo bench -p crossbook-core

# Local devnet: Postgres + Anvil.
dev:
    docker compose up -d

# One command demo: deploy, run the engine, and open the dashboard at :8080.
demo:
    bash scripts/demo.sh

deploy-local:
    cd contracts && forge script script/Deploy.s.sol --rpc-url $RPC_URL --broadcast

# Full end to end flow: brings up Postgres and Anvil, then runs the db and e2e tests.
e2e:
    docker compose up -d
    sleep 4
    cd contracts && forge build
    # One test thread: the e2e flows share one Anvil and deploy from the same
    # account, so they must not race on nonces. The differential test fuzzes the
    # Rust core against the live contract.
    DATABASE_URL=postgres://crossbook:crossbook@localhost:5432/crossbook RPC_URL=http://localhost:8545 cargo test -p crossbook-engine --features e2e --test e2e --test db --test differential -- --nocapture --test-threads=1
