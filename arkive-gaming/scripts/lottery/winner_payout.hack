// winner-payout-closure
// constructorInputs:
winner_pubkey:
    type: schnorr public key
asp_pubkey:
    type: schnorr public key
commitment1_hash:
    type: bytes32
commitment2_hash:
    type: bytes32
total_amount:
    type: little-endian uint64

// witness:
<winner_signature>
<asp_signature>
<commitment1_value>
<commitment1_nonce>
<commitment2_value>
<commitment2_nonce>

// script:
// Verify winner signature
OP_DATA_32
<winner_pubkey>
OP_CHECKSIGVERIFY

// Verify ASP signature
OP_DATA_32
<asp_pubkey>
OP_CHECKSIGVERIFY

// Verify commitment1 reveal
OP_DUP
OP_HASH256
OP_DATA_32
<commitment1_hash>
OP_EQUALVERIFY

// Verify commitment2 reveal
OP_DUP
OP_HASH256
OP_DATA_32
<commitment2_hash>
OP_EQUALVERIFY

// Verify output amount
OP_0
OP_INSPECTOUTPUTVALUE
OP_1
OP_EQUALVERIFY
OP_DATA_8
<total_amount>
OP_EQUAL

// miniscript policy
and_v(
    and_v(
        pk(<winner_pubkey>),
        pk(<asp_pubkey>)
    ),
    and_v(
        hash256_eq(<commitment1_value><commitment1_nonce>, <commitment1_hash>),
        and_v(
            hash256_eq(<commitment2_value><commitment2_nonce>, <commitment2_hash>),
            value_eq(out_value(0), <total_amount>)
        )
    )
)