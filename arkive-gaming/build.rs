use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=scripts/");

    // Validate that scripts exist for embedding
    let required_scripts = [
        "scripts/miniscript/lottery_escrow.miniscript",
        "scripts/miniscript/winner_payout.miniscript",
        "scripts/miniscript/timeout_refund.miniscript",
        "scripts/miniscript/mutual_abort.miniscript",
    ];

    for script in &required_scripts {
        if !Path::new(script).exists() {
            panic!("Required script not found for embedding: {}", script);
        }

        // Validate script is not empty
        let content = std::fs::read_to_string(script)
            .unwrap_or_else(|_| panic!("Failed to read script: {}", script));

        if content.trim().is_empty() {
            panic!("Script is empty: {}", script);
        }
    }

    println!(
        "cargo:warning=Successfully validated {} scripts for embedding",
        required_scripts.len()
    );
}
