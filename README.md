# Tokenizer-lib

[![Docs](https://docs.rs/tokenizer-lib/badge.svg)](https://docs.rs/tokenizer-lib/)
[![Crates](https://img.shields.io/crates/v/tokenizer-lib.svg)](https://crates.io/crates/tokenizer-lib)

Tokenization utilities for building parsers in Rust

### Examples

Buffered token channel:

```rust
let mut btq = BufferedTokenQueue::new();
btq.push(Token(12, Span(0, 2)));
btq.push(Token(32, Span(2, 4)));
btq.push(Token(52, Span(4, 8)));
assert_eq!(btq.next().unwrap(), Token(12, Span(0, 2)));
assert_eq!(btq.next().unwrap(), Token(32, Span(2, 4)));
assert_eq!(btq.next().unwrap(), Token(52, Span(4, 8)));
assert_eq!(btq.next(), None);
```

(Multi-thread safe) Parallel token queue:

```rust
let (mut sender, mut reader) = ParallelTokenQueue::new();
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

Generator token queue:

```rust
fn lexer(state: &mut u8, sender: &mut GeneratorTokenQueueBuffer<u8, ()>) {
    *state += 1;
    match state {
        1 | 2 | 3 => {
            sender.push(Token(*state * 2, ()))
        }
        _ => {}
    }
}

let mut reader = GeneratorTokenQueue::new(lexer, 0);

assert_eq!(reader.next().unwrap(), Token(2, ()));
assert_eq!(reader.next().unwrap(), Token(4, ()));
assert_eq!(reader.next().unwrap(), Token(6, ()));
assert!(reader.next().is_none());
```

Provides utilities such as [`peek`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#tymethod.peek), [`peek_n`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#tymethod.peek_n) and [`scan`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#tymethod.scan) for lookahead. Also [`expect_next`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#method.expect_next) for expecting a token value and [`conditional_next`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#method.conditional_next) for advancing on a predicate.