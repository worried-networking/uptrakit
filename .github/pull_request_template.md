<!-- markdownlint-disable MD041 -->

## Summary

- What changed?
- Why was it needed?

## Testing

- [ ] `cargo fmt --all`
- [ ] `cargo check --no-default-features --features db-sqlite`
- [ ] `cargo check --all-features`
- [ ] `cargo clippy --all-targets --no-default-features --features db-sqlite`
- [ ] `cargo clippy --all-targets --all-features`
- [ ] `cargo test --all-features`
- [ ] Frontend checks/build (if applicable)
- [ ] Integration tests (if applicable)

## Risk / Rollout Notes

- Migrations, config changes, security-sensitive areas, or operational caveats

## AI Assistance Disclosure

- Did AI materially assist with this change? If yes, say which tools were used and what they materially drafted or revised.
- Confirm that you reviewed, understood, and validated the final change before submitting it.
