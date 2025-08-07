// mutual-abort-closure
// constructorInputs:
player1_pubkey:
    type: schnorr public key
player2_pubkey:
    type: schnorr public key
asp_pubkey:
    type: schnorr public key
bet_amount:
    type: little-endian uint64

// witness:
<player1_signature>
<player2_signature>
<asp_signature>

// script:
// Verify all three signatures (mutual consent)
OP_DATA_32
<player1_pubkey>
OP_CHECKSIGVERIFY
OP_DATA_32
<player2_pubkey>
OP_CHECKSIGVERIFY
OP_DATA_32
<asp_pubkey>
OP_CHECKSIGVERIFY

// Verify refund outputs
OP_0
OP_INSPECTOUTPUTVALUE
OP_1
OP_EQUALVERIFY
OP_DATA_8
<bet_amount>
OP_EQUALVERIFY

OP_1
OP_INSPECTOUTPUTVALUE
OP_1
OP_EQUALVERIFY
OP_DATA_8
<bet_amount>
OP_EQUAL

// miniscript policy
and_v(
    and_v(
        pk(<player1_pubkey>),
        pk(<player2_pubkey>)
    ),
    and_v(
        pk(<asp_pubkey>),
        and_v(
            value_eq(out_value(0), <bet_amount>),
            value_eq(out_value(1), <bet_amount>)
        )
    )
)