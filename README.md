# Tokenizer-lib

[![Docs](https://docs.rs/tokenizer-lib/badge.svg)](https://docs.rs/tokenizer-lib/)
[![Crates](https://img.shields.io/crates/v/tokenizer-lib.svg)](https://crates.io/crates/tokenizer-lib)

Tokenization utilities for building parsers in Rust

### Examples

Buffered token channel:

```rust
use tokenizer_lib::{BufferedTokenQueue, Token, TokenReader, TokenSender, TokenTrait};

#[derive(PartialEq, Debug)]
struct Span(pub u32, pub u32);

#[derive(PartialEq, Debug)]
struct N(pub u32);

impl TokenTrait for N {}

let mut btq = BufferedTokenQueue::new();
btq.push(Token(N(12), Span(0, 2)));
btq.push(Token(N(32), Span(2, 4)));
btq.push(Token(N(52), Span(4, 8)));
assert_eq!(btq.next().unwrap().0, N(12));
assert_eq!(btq.next().unwrap().0, N(32));
assert_eq!(btq.next().unwrap().0, N(52));
assert!(btq.next().is_none());
```

(Multi-thread safe) Parallel token queue:

```rust
use tokenizer_lib::{ParallelTokenQueue, Token, TokenReader, TokenSender, TokenTrait};

#[derive(PartialEq, Debug)]
struct Span(pub u32, pub u32);

#[derive(PartialEq, Debug)]
struct N(pub u32);

impl TokenTrait for N {}

let (mut sender, mut reader) = ParallelTokenQueue::new();
std::thread::spawn(move || {
    sender.push(Token(N(12), Span(0, 2)));
    sender.push(Token(N(32), Span(2, 4)));
    sender.push(Token(N(52), Span(4, 8)));
});

assert_eq!(reader.next().unwrap().0, N(12));
assert_eq!(reader.next().unwrap().0, N(32));
assert_eq!(reader.next().unwrap().0, N(52));
assert!(reader.next().is_none());
```

Generator token queue:

```rust
use tokenizer_lib::{GeneratorTokenQueue, GeneratorTokenQueueBuffer, Token, TokenReader, TokenSender, TokenTrait};

#[derive(PartialEq, Debug)]
struct N(pub u32);

impl TokenTrait for N {}

fn lexer(state: &mut u32, sender: &mut GeneratorTokenQueueBuffer<N, ()>) {
    *state += 1;
    match state {
        1..=3 => {
            sender.push(Token(N(*state * 2), ()));
        }
        _ => {}
    }
}

let mut reader = GeneratorTokenQueue::new(lexer, 0);

assert_eq!(reader.next().unwrap().0, N(2));
assert_eq!(reader.next().unwrap().0, N(4));
assert_eq!(reader.next().unwrap().0, N(6));
assert!(reader.next().is_none());
```

Provides utilities such as [`peek`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#tymethod.peek), [`peek_n`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#tymethod.peek_n) and [`scan`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#tymethod.scan) for lookahead. Also [`expect_next`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#method.expect_next) for expecting a token value and [`conditional_next`](https://docs.rs/tokenizer-lib/latest/tokenizer_lib/trait.TokenReader.html#method.conditional_next) for advancing on a predicate.