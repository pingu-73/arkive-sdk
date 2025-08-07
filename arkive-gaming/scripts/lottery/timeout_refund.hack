// timeout-refund-closure
// constructorInputs:
player1_pubkey:
    type: schnorr public key
player2_pubkey:
    type: schnorr public key
asp_pubkey:
    type: schnorr public key
timeout_delay:
    type: BIP68 sequence
bet_amount:
    type: little-endian uint64

// witness:
<player1_signature>
<player2_signature>
<asp_signature>

// script:
// Check timeout has passed
OP_DATA_X
<timeout_delay>
OP_CHECKSEQUENCEVERIFY
OP_DROP

// Verify all signatures
OP_DATA_32
<player1_pubkey>
OP_CHECKSIGVERIFY
OP_DATA_32
<player2_pubkey>
OP_CHECKSIGVERIFY
OP_DATA_32
<asp_pubkey>
OP_CHECKSIGVERIFY

// Verify refund amounts (two outputs of bet_amount each)
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
    older(<timeout_delay>),
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
)