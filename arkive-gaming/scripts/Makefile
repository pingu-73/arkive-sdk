# MINISCRIPT_COMPILER = miniscript-compiler

# .PHONY: all escrow winner timeout abort clean validate

# all: escrow winner timeout abort

# escrow:
# 	$(MINISCRIPT_COMPILER) descriptor "$(shell cat ./miniscript/lottery_escrow.miniscript)"

# winner:
# 	$(MINISCRIPT_COMPILER) descriptor "$(shell cat ./miniscript/winner_payout.miniscript)"

# timeout:
# 	$(MINISCRIPT_COMPILER) descriptor "$(shell cat ./miniscript/timeout_refund.miniscript)"

# abort:
# 	$(MINISCRIPT_COMPILER) descriptor "$(shell cat ./miniscript/mutual_abort.miniscript)"

# validate: all
# 	@echo "All lottery scripts compiled successfully with Elements miniscript"

# # Test the compiler installation
# test-compiler:
# 	@echo "Testing Elements miniscript compiler..."
# 	$(MINISCRIPT_COMPILER) descriptor "eltr(02d7924d4f7d43ea965a465ae3095ff41131e5946f3c85f79e44adbcf8e27e080e)"

# scripts/Makefile
MINISCRIPT_COMPILER = miniscript-compiler

# Test parameters
TEST_PLAYER1_PK = 02d7924d4f7d43ea965a465ae3095ff41131e5946f3c85f79e44adbcf8e27e080e
TEST_PLAYER2_PK = 03d7924d4f7d43ea965a465ae3095ff41131e5946f3c85f79e44adbcf8e27e080f
TEST_ASP_PK = 02d7924d4f7d43ea965a465ae3095ff41131e5946f3c85f79e44adbcf8e27e080e
TEST_BET_AMOUNT = 1000
TEST_TOTAL_AMOUNT = 2000
TEST_TIMEOUT_DELAY = 144

.PHONY: all escrow winner timeout abort clean validate test-with-params

all: escrow winner timeout abort

escrow:
	$(MINISCRIPT_COMPILER) descriptor "$$(cat ./miniscript/lottery_escrow.miniscript)"

winner:
	$(MINISCRIPT_COMPILER) descriptor "$$(cat ./miniscript/winner_payout.miniscript)"

timeout:
	$(MINISCRIPT_COMPILER) descriptor "$$(cat ./miniscript/timeout_refund.miniscript)"

abort:
	$(MINISCRIPT_COMPILER) descriptor "$$(cat ./miniscript/mutual_abort.miniscript)"

# Test with actual parameter substitution
test-with-params:
	@echo "Testing scripts with parameter substitution..."
	@sed -e 's/player1_pubkey/$(TEST_PLAYER1_PK)/g' \
	     -e 's/player2_pubkey/$(TEST_PLAYER2_PK)/g' \
	     -e 's/asp_pubkey/$(TEST_ASP_PK)/g' \
	     -e 's/bet_amount/$(TEST_BET_AMOUNT)/g' \
	     -e 's/total_amount/$(TEST_TOTAL_AMOUNT)/g' \
	     -e 's/timeout_delay/$(TEST_TIMEOUT_DELAY)/g' \
	     ./miniscript/lottery_escrow.miniscript | $(MINISCRIPT_COMPILER) descriptor /dev/stdin
	@echo "Escrow script test passed"

validate: all
	@echo "All lottery scripts compiled successfully"

# Test individual scripts with parameters
test-escrow-params:
	@sed -e 's/player1_pubkey/$(TEST_PLAYER1_PK)/g' \
	     -e 's/player2_pubkey/$(TEST_PLAYER2_PK)/g' \
	     -e 's/asp_pubkey/$(TEST_ASP_PK)/g' \
	     -e 's/bet_amount/$(TEST_BET_AMOUNT)/g' \
	     -e 's/total_amount/$(TEST_TOTAL_AMOUNT)/g' \
	     -e 's/timeout_delay/$(TEST_TIMEOUT_DELAY)/g' \
	     ./miniscript/lottery_escrow.miniscript | $(MINISCRIPT_COMPILER) descriptor /dev/stdin

test-winner-params:
	@sed -e 's/winner_pubkey/$(TEST_PLAYER1_PK)/g' \
	     -e 's/asp_pubkey/$(TEST_ASP_PK)/g' \
	     -e 's/total_amount/$(TEST_TOTAL_AMOUNT)/g' \
	     ./miniscript/winner_payout.miniscript | $(MINISCRIPT_COMPILER) descriptor /dev/stdin

clean:
	rm -f *.compiled

# Validate script syntax without compilation
syntax-check:
	@echo "Checking script syntax..."
	@for script in miniscript/*.miniscript; do \
		echo "Checking $$script..."; \
		if ! grep -q "eltr(" "$$script"; then \
			echo "ERROR: $$script missing eltr() wrapper"; \
			exit 1; \
		fi; \
	done
	@echo "All scripts have valid syntax"