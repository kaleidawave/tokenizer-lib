# Tokenizer-lib

[![Docs](https://docs.rs/tokenizer-lib/badge.svg)](https://docs.rs/tokenizer-lib/)
[![Crates](https://img.shields.io/crates/v/tokenizer-lib.svg)](https://crates.io/crates/tokenizer-lib)

Tokenization utilities for building parsers in Rust

### Examples

Static token channel:

```rust
let mut stc = StaticTokenChannel::new();
stc.push(Token(12, Span(0, 2)));
stc.push(Token(32, Span(2, 4)));
stc.push(Token(52, Span(4, 8)));
assert_eq!(stc.next().unwrap(), Token(12, Span(0, 2)));
assert_eq!(stc.next().unwrap(), Token(32, Span(2, 4)));
assert_eq!(stc.next().unwrap(), Token(52, Span(4, 8)));
assert_eq!(stc.next(), None);
```

Streamed token channel:

```rust
let (mut sender, mut reader) = get_streamed_token_channel();
std::thread::spawn(move || {
    sender.push(Token(12, Span(0, 2)));
    sender.push(Token(32, Span(2, 4)));
    sender.push(Token(52, Span(4, 8)));
});

assert_eq!(reader.next().unwrap(), Token(12, Span(0, 2)));
assert_eq!(reader.next().unwrap(), Token(32, Span(2, 4)));
assert_eq!(reader.next().unwrap(), Token(52, Span(4, 8)));
assert_eq!(reader.next(), None);
```

Provides utilities such as `peek` and `scan` for lookahead. Also `expect_next` for expecting a token value.