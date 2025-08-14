use anyhow::Result;
use e2e_tests::*;
use serial_test::serial;
use tracing::info;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn test_basic_boarding_and_ark_transaction() -> Result<()> {
    let env = setup_test_environment().await?;

    info!("Starting basic boarding and Ark transaction test");

    let alice = TestWallet::create(&env.manager, "alice").await?;
    let bob = TestWallet::create(&env.manager, "bob").await?;

    info!("Created wallets for Alice and Bob");

    let bob_ark_addr = bob.get_ark_address().await?;
    info!("Bob Ark address: {}", bob_ark_addr);

    // Alice boards funds
    let funding_amount = 1.0;
    let round_result = alice.board_funds(&env, funding_amount).await?;
    info!(
        "Alice successfully boarded funds in round: {}",
        round_result
    );

    alice.wait_for_balance(100_000_000, 0, 60).await?;

    let (alice_confirmed, alice_pending) = alice.get_balance().await?;
    info!(
        "Alice balance after boarding: {} confirmed, {} pending",
        alice_confirmed.to_sat(),
        alice_pending.to_sat()
    );

    // Alice sends some sats to Bob
    let send_amount = 50_000_000;
    info!("Alice sending {} sats to Bob...", send_amount);

    let tx_id = alice.send_ark(&bob_ark_addr, send_amount).await?;
    info!("Transaction sent: {}", tx_id);

    let expected_alice_remaining = 100_000_000 - send_amount - ARK_TRANSACTION_FEE;
    alice
        .wait_for_balance(0, expected_alice_remaining, 30)
        .await?;

    let (alice_after_send_confirmed, alice_after_send_pending) = alice.get_balance().await?;
    info!(
        "Alice balance after sending: {} confirmed, {} pending",
        alice_after_send_confirmed.to_sat(),
        alice_after_send_pending.to_sat()
    );

    info!("Bob waiting to receive transaction...");
    bob.wait_for_incoming_transaction(send_amount, 60).await?;

    let (bob_confirmed, bob_pending) = bob.get_balance().await?;
    info!(
        "Bob balance after receiving: {} confirmed, {} pending",
        bob_confirmed.to_sat(),
        bob_pending.to_sat()
    );

    assert_eq!(
        bob_pending.to_sat(),
        send_amount,
        "Bob should receive exactly what Alice sent"
    );

    info!("Bob participating in round to confirm VTXO...");

    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    let bob_round_result = bob.participate_in_round().await?;
    assert!(
        bob_round_result.is_some(),
        "Bob should have participated in a round"
    );

    // Wait for Bob to have confirmed balance
    bob.wait_for_balance(send_amount, 0, 60).await?;

    let (bob_final_confirmed, bob_final_pending) = bob.get_balance().await?;
    info!(
        "Bob final balance: {} confirmed, {} pending",
        bob_final_confirmed.to_sat(),
        bob_final_pending.to_sat()
    );

    assert_eq!(bob_final_confirmed.to_sat(), send_amount);
    assert_eq!(bob_final_pending.to_sat(), 0);

    info!("Alice participating in round...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let alice_round_result = alice.participate_in_round().await?;
    if alice_round_result.is_some() {
        info!("Alice participated in round to confirm change");
        alice
            .wait_for_balance(expected_alice_remaining, 0, 60)
            .await?;
    } else {
        info!("Alice didn't need to participate in round (change already confirmed)");
    }

    let (alice_final_confirmed, alice_final_pending) = alice.get_balance().await?;
    let alice_total = alice_final_confirmed.to_sat() + alice_final_pending.to_sat();

    let total_remaining = alice_total + bob_final_confirmed.to_sat() + ARK_TRANSACTION_FEE;
    assert_eq!(
        total_remaining, 100_000_000,
        "Total funds should be conserved (minus fees)"
    );

    info!("  Basic boarding and Ark transaction test completed successfully!");
    info!("  Final state:");
    info!(
        "  Alice: {} sats (confirmed: {}, pending: {})",
        alice_total,
        alice_final_confirmed.to_sat(),
        alice_final_pending.to_sat()
    );
    info!("  Bob: {} sats", bob_final_confirmed.to_sat());
    info!("  Fees paid: {} sats", ARK_TRANSACTION_FEE);

    Ok(())
}