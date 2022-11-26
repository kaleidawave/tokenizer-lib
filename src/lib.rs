//! Tokenization utilities for building parsers in Rust
#![allow(clippy::type_complexity, clippy::new_ret_no_self)]

use std::{
    collections::VecDeque,
    fmt::{self, Debug},
    usize,
};

pub use buffered_token_queue::*;
pub use generator_token_queue::*;
#[cfg(not(target_arch = "wasm32"))]
pub use parallel_token_queue::*;

/// [PartialEq] is required for comparing tokens with [TokenReader::expect_next]
pub trait TokenTrait: PartialEq {
    /// Use this for *nully* tokens. Will be skipped [TokenReader::expect_next]
    fn is_skippable(&self) -> bool {
        false
    }
}

/// A structure with a piece of data and some additional data such as a position
pub struct Token<T: TokenTrait, TData>(pub T, pub TData);

impl<T: TokenTrait + Debug, TData: Debug> Debug for Token<T, TData> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Token")
            .field(&self.0)
            .field(&self.1)
            .finish()
    }
}

/// A *reader* over a sequence of tokens
pub trait TokenReader<T: TokenTrait, TData> {
    /// Returns a reference to next token but does not advance current position
    fn peek(&mut self) -> Option<&Token<T, TData>>;

    /// Returns a reference to nth (zero based) upcoming token without advancing
    fn peek_n(&mut self, n: usize) -> Option<&Token<T, TData>>;

    /// Returns the next token and advances
    fn next(&mut self) -> Option<Token<T, TData>>;

    /// Returns next if `cb` returns true for the upcoming token (the token from [TokenReader::peek])
    fn conditional_next(&mut self, cb: impl FnOnce(&T) -> bool) -> Option<Token<T, TData>> {
        let peek = self.peek()?;
        if cb(&peek.0) {
            self.next()
        } else {
            None
        }
    }

    /// Runs the closure (cb) over upcoming tokens. Passes the value behind the Token to the closure.
    /// Will stop and return a reference **to the next Token from when the closure returns true**.
    /// Returns None if scanning finishes before closure returns true. Does not advance the reader.
    ///
    /// Used for lookahead and then branching based on return value during parsing
    fn scan(&mut self, cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>>;

    /// Tests that next token matches an expected type. Will return error if does not
    /// match. The `Ok` value contains the data of the valid token.
    /// Else it will return the Err with the expected token type and the token that did not match
    ///
    /// Is the token is skippable (using [TokenTrait::is_skippable])
    fn expect_next(&mut self, expected_type: T) -> Result<TData, Option<(T, Token<T, TData>)>> {
        match self.next() {
            Some(token) => {
                if token.0 == expected_type {
                    Ok(token.1)
                } else if token.0.is_skippable() {
                    // This will advance to the next, won't cyclically recurse
                    self.expect_next(expected_type)
                } else {
                    Err(Some((expected_type, token)))
                }
            }
            None => Err(None),
        }
    }
}

/// Trait for a sender that can append a token to a sequence
pub trait TokenSender<T: TokenTrait, TData> {
    /// Appends a new [`Token`]
    /// Will return false if could not push token
    fn push(&mut self, token: Token<T, TData>) -> bool;
}

mod buffered_token_queue {
    use super::*;
    /// A queue which can be used as a sender and reader. Use this for buffering all the tokens before reading
    pub struct BufferedTokenQueue<T: TokenTrait, TData> {
        buffer: VecDeque<Token<T, TData>>,
    }

    impl<T: TokenTrait, TData> Default for BufferedTokenQueue<T, TData> {
        fn default() -> Self {
            Self {
                buffer: Default::default(),
            }
        }
    }

    impl<T: TokenTrait, TData> BufferedTokenQueue<T, TData> {
        /// Constructs a new [`BufferedTokenQueue`]
        pub fn new() -> Self {
            Default::default()
        }
    }

    impl<T: TokenTrait, TData> TokenSender<T, TData> for BufferedTokenQueue<T, TData> {
        fn push(&mut self, token: Token<T, TData>) -> bool {
            self.buffer.push_back(token);
            true
        }
    }

    impl<T: TokenTrait, TData> TokenReader<T, TData> for BufferedTokenQueue<T, TData> {
        fn peek(&mut self) -> Option<&Token<T, TData>> {
            self.buffer.front()
        }

        fn peek_n(&mut self, n: usize) -> Option<&Token<T, TData>> {
            self.buffer.get(n)
        }

        fn next(&mut self) -> Option<Token<T, TData>> {
            self.buffer.pop_front()
        }

        fn scan(&mut self, mut cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>> {
            let mut iter = self.buffer.iter().peekable();
            while let Some(token) = iter.next() {
                if cb(&token.0, &token.1) {
                    return iter.peek().copied();
                }
            }
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod parallel_token_queue {
    use super::*;
    use std::sync::mpsc::{sync_channel, Receiver, RecvError, SyncSender};

    const DEFAULT_BUFFER_SIZE: usize = 20;

    /// A token queue used for doing lexing and parsing on different threads. Will send tokens between threads
    pub struct ParallelTokenQueue;

    impl ParallelTokenQueue {
        /// Creates two items, a sender and a receiver. Where the reader is on the parsing thread and the
        /// sender is on the lexer thread
        pub fn new<T: TokenTrait, TData>(
        ) -> (ParallelTokenSender<T, TData>, ParallelTokenReader<T, TData>) {
            Self::new_with_buffer_size(DEFAULT_BUFFER_SIZE)
        }

        pub fn new_with_buffer_size<T: TokenTrait, TData>(
            buffer_size: usize,
        ) -> (ParallelTokenSender<T, TData>, ParallelTokenReader<T, TData>) {
            let (sender, receiver) = sync_channel::<Token<T, TData>>(buffer_size);

            (
                ParallelTokenSender(sender),
                ParallelTokenReader {
                    receiver,
                    cache: VecDeque::new(),
                },
            )
        }
    }

    // Sender and reader structs generate by `ParallelTokenQueue::new`:

    #[doc(hidden)]
    pub struct ParallelTokenSender<T: TokenTrait, TData>(SyncSender<Token<T, TData>>);

    #[doc(hidden)]
    pub struct ParallelTokenReader<T: TokenTrait, TData> {
        receiver: Receiver<Token<T, TData>>,
        cache: VecDeque<Token<T, TData>>,
    }

    impl<T: TokenTrait, TData> TokenSender<T, TData> for ParallelTokenSender<T, TData> {
        fn push(&mut self, token: Token<T, TData>) -> bool {
            self.0.send(token).is_ok()
        }
    }

    impl<T: TokenTrait, TData> TokenReader<T, TData> for ParallelTokenReader<T, TData> {
        fn peek(&mut self) -> Option<&Token<T, TData>> {
            if self.cache.is_empty() {
                match self.receiver.recv() {
                    Ok(token) => self.cache.push_back(token),
                    // Err is reader has dropped e.g. no more tokens
                    Err(RecvError) => {
                        return None;
                    }
                }
            }
            self.cache.front()
        }

        fn peek_n(&mut self, n: usize) -> Option<&Token<T, TData>> {
            while self.cache.len() <= n {
                match self.receiver.recv() {
                    Ok(token) => self.cache.push_back(token),
                    // Err is reader has dropped e.g. no more tokens
                    Err(RecvError) => {
                        return None;
                    }
                }
            }
            self.cache.get(n)
        }

        fn next(&mut self) -> Option<Token<T, TData>> {
            if !self.cache.is_empty() {
                return self.cache.pop_front();
            }
            self.receiver.recv().ok()
        }

        fn scan(&mut self, mut cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>> {
            let found = scan_cache(&mut self.cache, &mut cb);
            let mut return_next = match found {
                ScanCacheResult::RetrievableInCacheAt(idx) => return self.cache.get(idx),
                ScanCacheResult::Found => true,
                ScanCacheResult::NotFound => false,
            };
            loop {
                match self.receiver.recv() {
                    Ok(val) => {
                        if return_next {
                            self.cache.push_back(val);
                            return self.cache.back();
                        }
                        if cb(&val.0, &val.1) {
                            return_next = true;
                        }
                        self.cache.push_back(val);
                    }
                    // Err is reader has dropped e.g. no more tokens
                    Err(RecvError) => {
                        return None;
                    }
                }
            }
        }
    }
}

mod generator_token_queue {
    use super::*;

    /// A token queue which has a backing generator/lexer which is called when needed by parsing logic
    pub struct GeneratorTokenQueue<T, TData, TGeneratorState, TGenerator>
    where
        T: TokenTrait,
        for<'a> TGenerator:
            FnMut(&mut TGeneratorState, &mut GeneratorTokenQueueBuffer<'a, T, TData>),
    {
        generator: TGenerator,
        generator_state: TGeneratorState,
        cache: VecDeque<Token<T, TData>>,
    }

    /// A wrapping struct for the cache around [`GeneratorTokenQueue`]. Use as the second parameter
    /// in the generator/lexer function
    pub struct GeneratorTokenQueueBuffer<'a, T: TokenTrait, TData>(
        &'a mut VecDeque<Token<T, TData>>,
    );

    impl<'a, T: TokenTrait, TData> GeneratorTokenQueueBuffer<'a, T, TData> {
        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        pub fn len(&self) -> usize {
            self.0.len()
        }
    }

    impl<'a, T: TokenTrait, TData> TokenSender<T, TData> for GeneratorTokenQueueBuffer<'a, T, TData> {
        fn push(&mut self, token: Token<T, TData>) -> bool {
            self.0.push_back(token);
            true
        }
    }

    impl<T, TData, TGeneratorState, TGenerator>
        GeneratorTokenQueue<T, TData, TGeneratorState, TGenerator>
    where
        T: TokenTrait,
        for<'a> TGenerator:
            FnMut(&mut TGeneratorState, &mut GeneratorTokenQueueBuffer<'a, T, TData>),
    {
        /// Create a new [`GeneratorTokenQueue`] with a lexer function and initial state
        pub fn new(generator: TGenerator, generator_state: TGeneratorState) -> Self {
            GeneratorTokenQueue {
                generator,
                generator_state,
                cache: VecDeque::new(),
            }
        }
    }

    impl<T, TData, TGeneratorState, TGenerator> TokenReader<T, TData>
        for GeneratorTokenQueue<T, TData, TGeneratorState, TGenerator>
    where
        T: TokenTrait,
        TData: Debug,
        for<'a> TGenerator:
            FnMut(&mut TGeneratorState, &mut GeneratorTokenQueueBuffer<'a, T, TData>),
    {
        fn peek(&mut self) -> Option<&Token<T, TData>> {
            if self.cache.is_empty() {
                (self.generator)(
                    &mut self.generator_state,
                    &mut GeneratorTokenQueueBuffer(&mut self.cache),
                );
            }
            self.cache.front()
        }

        fn peek_n(&mut self, n: usize) -> Option<&Token<T, TData>> {
            while self.cache.len() <= n {
                (self.generator)(
                    &mut self.generator_state,
                    &mut GeneratorTokenQueueBuffer(&mut self.cache),
                );
            }
            self.cache.get(n)
        }

        fn next(&mut self) -> Option<Token<T, TData>> {
            if !self.cache.is_empty() {
                return self.cache.pop_front();
            }
            (self.generator)(
                &mut self.generator_state,
                &mut GeneratorTokenQueueBuffer(&mut self.cache),
            );
            self.cache.pop_front()
        }

        fn scan(&mut self, mut cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>> {
            let cb = &mut cb;
            let found = scan_cache(&mut self.cache, cb);
            let mut return_next = match found {
                ScanCacheResult::RetrievableInCacheAt(idx) => return self.cache.get(idx),
                ScanCacheResult::Found => true,
                ScanCacheResult::NotFound => false,
            };
            let mut found = None::<usize>;
            while found.is_none() {
                let start = self.cache.len();
                (self.generator)(
                    &mut self.generator_state,
                    &mut GeneratorTokenQueueBuffer(&mut self.cache),
                );
                if self.cache.is_empty() {
                    return None;
                }
                for (idx, token) in self.cache.iter().enumerate().skip(start) {
                    if return_next {
                        found = Some(idx);
                        break;
                    }
                    if cb(&token.0, &token.1) {
                        return_next = true;
                    }
                }
            }
            self.cache.get(found.unwrap())
        }
    }
}

enum ScanCacheResult {
    RetrievableInCacheAt(usize),
    NotFound,
    // Aka pull out next one
    Found,
}

/// Returns the idx of the **next item** after cb returns true
/// This returns the idx instead of the item for lifetime reasons
fn scan_cache<T: TokenTrait, TData>(
    cache: &mut VecDeque<Token<T, TData>>,
    cb: &mut impl FnMut(&T, &TData) -> bool,
) -> ScanCacheResult {
    let mut cb_returned_true_at_idx = None::<usize>;
    // Try to find in idx. Returns the idx when found
    for (idx, token) in cache.iter().enumerate() {
        if cb(&token.0, &token.1) {
            cb_returned_true_at_idx = Some(idx);
            break;
        }
    }
    if let Some(idx) = cb_returned_true_at_idx {
        if idx + 1 < cache.len() {
            ScanCacheResult::RetrievableInCacheAt(idx + 1)
        } else {
            ScanCacheResult::Found
        }
    } else {
        ScanCacheResult::NotFound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl<T: TokenTrait, TData: PartialEq> PartialEq for Token<T, TData> {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0 && self.1 == other.1
        }
    }

    impl<T: TokenTrait, TData: PartialEq> Eq for Token<T, TData> {}

    impl TokenTrait for u32 {}

    mod buffered_token_queue {
        use super::{BufferedTokenQueue, Token, TokenReader, TokenSender};

        #[test]
        fn next() {
            let mut btq = BufferedTokenQueue::new();
            btq.push(Token(12, ()));
            btq.push(Token(32, ()));
            btq.push(Token(52, ()));

            assert_eq!(btq.next().unwrap(), Token(12, ()));
            assert_eq!(btq.next().unwrap(), Token(32, ()));
            assert_eq!(btq.next().unwrap(), Token(52, ()));
            assert!(btq.next().is_none());
        }

        #[test]
        fn peek() {
            let mut btq = BufferedTokenQueue::new();
            btq.push(Token(12, ()));

            assert_eq!(btq.peek().unwrap(), &Token(12, ()));
            assert_eq!(btq.next().unwrap(), Token(12, ()));
            assert!(btq.next().is_none());
        }

        #[test]
        fn peek_n() {
            let mut btq = BufferedTokenQueue::new();
            btq.push(Token(12, ()));
            btq.push(Token(32, ()));
            btq.push(Token(52, ()));

            assert_eq!(btq.peek_n(2).unwrap(), &Token(52, ()));
            assert_eq!(btq.next().unwrap(), Token(12, ()));
            assert_eq!(btq.next().unwrap(), Token(32, ()));
            assert_eq!(btq.next().unwrap(), Token(52, ()));
            assert!(btq.next().is_none());
        }

        #[test]
        fn expect_next() {
            let mut btq = BufferedTokenQueue::new();
            btq.push(Token(12, ()));
            btq.push(Token(24, ()));

            assert_eq!(btq.expect_next(12).unwrap(), ());
            assert!(btq.expect_next(10).is_err());
            assert!(btq.next().is_none());
        }

        #[test]
        fn scan() {
            let mut btq = BufferedTokenQueue::new();
            for val in vec![4, 10, 100, 200] {
                btq.push(Token(val, ()));
            }

            let mut count = 0;
            let x = btq.scan(move |token_val, _| {
                count += token_val;
                count > 100
            });
            assert_eq!(x.unwrap().0, 200);

            let mut count = 0;
            let y = btq.scan(move |token_val, _| {
                count += token_val;
                count > 1000
            });
            assert_eq!(y, None);

            assert_eq!(btq.next().unwrap().0, 4);
            assert_eq!(btq.next().unwrap().0, 10);
            assert_eq!(btq.next().unwrap().0, 100);
            assert_eq!(btq.next().unwrap().0, 200);
            assert!(btq.next().is_none());
        }
    }

    mod parallel_token_queue {
        use super::{ParallelTokenQueue, Token, TokenReader, TokenSender};

        #[test]
        fn next() {
            let (mut sender, mut reader) = ParallelTokenQueue::new();
            std::thread::spawn(move || {
                sender.push(Token(12, ()));
                sender.push(Token(32, ()));
                sender.push(Token(52, ()));
            });

            assert_eq!(reader.next().unwrap(), Token(12, ()));
            assert_eq!(reader.next().unwrap(), Token(32, ()));
            assert_eq!(reader.next().unwrap(), Token(52, ()));
            assert!(reader.next().is_none());
        }

        #[test]
        fn peek() {
            let (mut sender, mut reader) = ParallelTokenQueue::new();
            std::thread::spawn(move || {
                sender.push(Token(12, ()));
            });

            assert_eq!(reader.peek().unwrap(), &Token(12, ()));
            assert_eq!(reader.next().unwrap(), Token(12, ()));
            assert!(reader.next().is_none());
        }

        #[test]
        fn next_n() {
            let (mut sender, mut reader) = ParallelTokenQueue::new();
            std::thread::spawn(move || {
                sender.push(Token(12, ()));
                sender.push(Token(32, ()));
                sender.push(Token(52, ()));
            });

            assert_eq!(reader.peek_n(2).unwrap(), &Token(52, ()));
            assert_eq!(reader.next().unwrap(), Token(12, ()));
            assert_eq!(reader.next().unwrap(), Token(32, ()));
            assert_eq!(reader.next().unwrap(), Token(52, ()));
            assert!(reader.next().is_none());
        }

        #[test]
        fn expect_next() {
            let (mut sender, mut reader) = ParallelTokenQueue::new();
            std::thread::spawn(move || {
                sender.push(Token(12, ()));
                sender.push(Token(24, ()));
            });

            assert_eq!(reader.expect_next(12).unwrap(), ());
            assert!(reader.expect_next(10).is_err());
            assert!(reader.next().is_none());
        }

        #[test]
        fn scan() {
            let (mut sender, mut reader) = ParallelTokenQueue::new();
            std::thread::spawn(move || {
                for val in vec![4, 10, 100, 200] {
                    sender.push(Token(val, ()));
                }
            });

            let mut count = 0;
            let x = reader.scan(move |token_val, _| {
                count += token_val;
                count > 100
            });
            assert_eq!(x.unwrap().0, 200);

            let mut count = 0;
            let y = reader.scan(move |token_val, _| {
                count += token_val;
                count > 1000
            });
            assert_eq!(y, None);
            assert_eq!(reader.next().unwrap().0, 4);
            assert_eq!(reader.next().unwrap().0, 10);
            assert_eq!(reader.next().unwrap().0, 100);
            assert_eq!(reader.next().unwrap().0, 200);
            assert!(reader.next().is_none());
        }
    }

    mod generator_token_queue {
        use super::{
            GeneratorTokenQueue, GeneratorTokenQueueBuffer, Token, TokenReader, TokenSender,
        };

        fn lexer(state: &mut u32, sender: &mut GeneratorTokenQueueBuffer<u32, ()>) {
            *state += 1;
            match state {
                1 | 2 | 3 => {
                    sender.push(Token(*state * 2, ()));
                }
                _ => {}
            }
        }

        #[test]
        fn next() {
            let mut reader = GeneratorTokenQueue::new(lexer, 0);

            assert_eq!(reader.next().unwrap(), Token(2, ()));
            assert_eq!(reader.next().unwrap(), Token(4, ()));
            assert_eq!(reader.next().unwrap(), Token(6, ()));
            assert!(reader.next().is_none());
        }

        #[test]
        fn peek() {
            let mut reader = GeneratorTokenQueue::new(lexer, 0);
            assert_eq!(reader.peek().unwrap(), &Token(2, ()));
            assert_eq!(reader.next().unwrap(), Token(2, ()));
        }

        #[test]
        fn peek_n() {
            let mut reader = GeneratorTokenQueue::new(lexer, 0);

            assert_eq!(reader.peek_n(2).unwrap(), &Token(6, ()));
            assert_eq!(reader.next().unwrap(), Token(2, ()));
            assert_eq!(reader.next().unwrap(), Token(4, ()));
            assert_eq!(reader.next().unwrap(), Token(6, ()));
            assert!(reader.next().is_none());
        }

        #[test]
        fn expect_next() {
            let mut reader = GeneratorTokenQueue::new(lexer, 0);

            assert!(reader.expect_next(2).is_ok());
            assert!(reader.expect_next(5).is_err());
            assert!(reader.expect_next(6).is_ok());
        }

        #[test]
        fn scan() {
            let mut reader = GeneratorTokenQueue::new(lexer, 0);

            let mut count = 0;
            let x = reader.scan(move |token_val, _| {
                count += token_val;
                count > 3
            });
            assert_eq!(x.unwrap().0, 6);
            assert_eq!(reader.next().unwrap(), Token(2, ()));
        }
    }

    #[test]
    fn conditional_next() {
        let mut btq = BufferedTokenQueue::new();
        btq.push(Token(12, ()));
        btq.push(Token(32, ()));

        assert_eq!(btq.conditional_next(|t| *t == 28), None);
        assert_eq!(btq.conditional_next(|t| *t == 12), Some(Token(12, ())));
        assert_eq!(btq.next().unwrap(), Token(32, ()));
        assert!(btq.next().is_none());
    }

    mod skippable_token {
        use super::{BufferedTokenQueue, Token, TokenReader, TokenSender, TokenTrait};

        #[derive(PartialEq, Eq, Debug)]
        enum TokenType {
            A,
            B,
            Ignore,
        }

        impl TokenTrait for TokenType {
            fn is_skippable(&self) -> bool {
                matches!(self, TokenType::Ignore)
            }
        }

        #[test]
        fn still_show_with_next() {
            let mut btq = BufferedTokenQueue::new();
            generate_tokens(&mut btq);

            assert_eq!(btq.next().unwrap(), Token(TokenType::A, ()));
            assert_eq!(btq.next().unwrap(), Token(TokenType::Ignore, ()));
            assert_eq!(btq.next().unwrap(), Token(TokenType::B, ()));
            assert!(btq.next().is_none());
        }

        #[test]
        fn skipped_under_expect_next() {
            let mut btq = BufferedTokenQueue::new();
            generate_tokens(&mut btq);

            assert!(btq.expect_next(TokenType::A).is_ok());
            assert!(btq.expect_next(TokenType::B).is_ok());
            assert!(btq.next().is_none());
        }

        fn generate_tokens(btq: &mut BufferedTokenQueue<TokenType, ()>) {
            btq.push(Token(TokenType::A, ()));
            btq.push(Token(TokenType::Ignore, ()));
            btq.push(Token(TokenType::B, ()));
        }
    }
}
