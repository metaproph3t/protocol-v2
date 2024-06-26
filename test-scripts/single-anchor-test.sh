if [ "$1" != "--skip-build" ]
  then
    anchor build -- --features anchor-test && anchor test &&
    cp target/idl/drift.json sdk/src/idl/
fi

export ANCHOR_WALLET=~/.config/solana/id.json

test_files=(tokenFaucet.ts)

for test_file in ${test_files[@]}; do
  ts-mocha -t 300000 ./tests/${test_file} || exit 1
done 