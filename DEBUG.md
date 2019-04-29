# Useful Debugging Notes

## ASAN

gdb: `break __sanitizer::Die

## TSAN

Cargo.toml feature: `sanitize-thread`
RUSTFLAGS=-Z sanitizer=thread
