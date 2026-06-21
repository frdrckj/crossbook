#!/usr/bin/env bash
# One command demo: brings up Postgres and Anvil, deploys the settlement contract
# and two funded demo tokens, then runs the engine with the dashboard at
# http://localhost:8080. Stop with Ctrl-C.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.foundry/bin:$PATH"

# Anvil's public test keys (local only).
DEPLOYER=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
MAKER_A=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
MAKER_B=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a
RPC=http://localhost:8545
DB=postgres://crossbook:crossbook@localhost:5432/crossbook

echo "==> Postgres"
docker compose up -d postgres

if ! cast block-number --rpc-url "$RPC" >/dev/null 2>&1; then
  echo "==> Anvil"
  anvil --silent --chain-id 31337 &
  sleep 2
fi

echo "==> Deploying demo contracts and tokens"
out=$(cd contracts && DEPLOYER_PRIVATE_KEY=$DEPLOYER MAKER_A_KEY=$MAKER_A MAKER_B_KEY=$MAKER_B \
  forge script script/DeployDemo.s.sol --rpc-url "$RPC" --broadcast -vv)
SETTLEMENT=$(echo "$out" | grep -oE 'SETTLEMENT_ADDRESS=0x[a-fA-F0-9]{40}' | head -1 | cut -d= -f2)
TOKEN_A=$(echo "$out" | grep -oE 'DEMO_TOKEN_A=0x[a-fA-F0-9]{40}' | head -1 | cut -d= -f2)
TOKEN_B=$(echo "$out" | grep -oE 'DEMO_TOKEN_B=0x[a-fA-F0-9]{40}' | head -1 | cut -d= -f2)
echo "    settlement $SETTLEMENT"
echo "    token A    $TOKEN_A"
echo "    token B    $TOKEN_B"

echo "==> Engine and dashboard on http://localhost:8080"
SQLX_OFFLINE=true \
  DATABASE_URL=$DB RPC_URL=$RPC \
  SETTLEMENT_ADDRESS=$SETTLEMENT SOLVER_PRIVATE_KEY=$DEPLOYER \
  DEMO_TOKEN_A=$TOKEN_A DEMO_TOKEN_B=$TOKEN_B BIND=127.0.0.1:8080 \
  cargo run -p crossbook-engine
