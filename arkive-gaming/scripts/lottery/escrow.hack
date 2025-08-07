// lottery-escrow-closure
// constructorInputs:
player1_pubkey:
    type: schnorr public key
player2_pubkey:
    type: schnorr public key
asp_pubkey:
    type: schnorr public key
bet_amount:
    type: little-endian uint64
commitment1_hash:
    type: bytes32
commitment2_hash:
    type: bytes32

// witness (empty for introspection)

// script:
// Verify this is a proper escrow setup
OP_0
OP_INSPECTOUTPUTVALUE
OP_1
OP_EQUALVERIFY
OP_DATA_8
<bet_amount>
OP_2
OP_MUL64                            // stack: [overflow, bet_amount * 2]
OP_1
OP_EQUALVERIFY                      // stack: [bet_amount * 2]
OP_EQUAL                            // Verify output value equals 2 * bet_amount

// miniscript policy
value_eq(out_value(0), mul(<bet_amount>, 2))