//! Tokenization utilities for building parsers in Rust

use std::{collections::VecDeque, fmt::Debug, usize};

pub use buffered_token_queue::*;
pub use generator_token_queue::*;
#[cfg(not(target_arch = "wasm32"))]
pub use parallel_token_queue::*;

/// A structure with a piece of data and some additional data such as a position
pub struct Token<T: PartialEq, TData>(pub T, pub TData);

/// A *reader* over a sequence of tokens
pub trait TokenReader<T: PartialEq, TData> {
    /// Returns a reference to next token but does not advance current position
    fn peek(&mut self) -> Option<&Token<T, TData>>;

    /// Returns the next token and advances
    fn next(&mut self) -> Option<Token<T, TData>>;

    /// Runs the closure (cb) over upcoming tokens. Passes the value behind the Token to the closure.
    /// Will stop and return a reference **to the next Token from when the closure returns true**.
    /// Returns None if scanning finishes before closure returns true. Does not advance the reader.
    ///
    /// Used for lookahead and then branching based on return value during parsing
    fn scan(&mut self, cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>>;

    /// Tests that next token matches an expected type. Will return error if does not
    /// match. The `Ok` value contains the data of the valid token.
    /// Else it will return the Err with the expected token type and the token that did not match
    fn expect_next(&mut self, expected_type: T) -> Result<TData, Option<(T, Token<T, TData>)>> {
        match self.next() {
            Some(token) => {
                if &token.0 != &expected_type {
                    Err(Some((expected_type, token)))
                } else {
                    Ok(token.1)
                }
            }
            None => Err(None),
        }
    }
}

/// Trait for a sender that can append a token to a sequence
pub trait TokenSender<T: PartialEq, TData> {
    /// Appends a new [`Token`]
    /// Will return false if could not push token
    fn push(&mut self, token: Token<T, TData>) -> bool;
}

mod buffered_token_queue {
    use super::*;
    /// A queue which can be used as a sender and reader. Use this for buffering all the tokens before reading
    pub struct BufferedTokenQueue<T: PartialEq, TData> {
        buffer: VecDeque<Token<T, TData>>,
    }

    impl<T: PartialEq, TData> BufferedTokenQueue<T, TData> {
        /// Constructs a new [`BufferedTokenQueue`]
        pub fn new() -> Self {
            BufferedTokenQueue {
                buffer: VecDeque::new(),
            }
        }
    }

    impl<T: PartialEq, TData> TokenSender<T, TData> for BufferedTokenQueue<T, TData> {
        fn push(&mut self, token: Token<T, TData>) -> bool {
            self.buffer.push_back(token);
            true
        }
    }

    impl<T: PartialEq, TData> TokenReader<T, TData> for BufferedTokenQueue<T, TData> {
        fn peek(&mut self) -> Option<&Token<T, TData>> {
            self.buffer.front()
        }

        fn next(&mut self) -> Option<Token<T, TData>> {
            self.buffer.pop_front()
        }

        fn scan(&mut self, mut cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>> {
            let mut iter = self.buffer.iter().peekable();
            while let Some(token) = iter.next() {
                if cb(&token.0, &token.1) {
                    return iter.peek().map(|v| *v);
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
        pub fn new<T: PartialEq, TData>(
        ) -> (ParallelTokenSender<T, TData>, ParallelTokenReader<T, TData>) {
            Self::new_with_buffer_size(DEFAULT_BUFFER_SIZE)
        }

        pub fn new_with_buffer_size<T: PartialEq, TData>(
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
    pub struct ParallelTokenSender<T: PartialEq, TData>(SyncSender<Token<T, TData>>);

    #[doc(hidden)]
    pub struct ParallelTokenReader<T: PartialEq, TData> {
        receiver: Receiver<Token<T, TData>>,
        cache: VecDeque<Token<T, TData>>,
    }

    impl<T: PartialEq, TData> TokenSender<T, TData> for ParallelTokenSender<T, TData> {
        fn push(&mut self, token: Token<T, TData>) -> bool {
            self.0.send(token).is_ok()
        }
    }

    impl<T: PartialEq, TData> TokenReader<T, TData> for ParallelTokenReader<T, TData> {
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

        fn next(&mut self) -> Option<Token<T, TData>> {
            if !self.cache.is_empty() {
                return self.cache.pop_front();
            }
            self.receiver.recv().ok()
        }

        fn scan(&mut self, mut cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>> {
            let found = scan_cache(&mut self.cache, &mut cb);
            let mut return_next = match found {
                ScanCacheResult::FoundInCache(i) => return self.cache.get(i),
                ScanCacheResult::NotFound => false,
                ScanCacheResult::Found => true,
            };
            loop {
                match self.receiver.recv() {
                    Ok(val) => {
                        if return_next {
                            self.cache.push_back(val);
                            return self.cache.back();
                        }
                        if (&mut cb)(&val.0, &val.1) {
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
        T: PartialEq,
        for<'a> TGenerator:
            FnMut(&mut TGeneratorState, &mut GeneratorTokenQueueBuffer<'a, T, TData>) -> (),
    {
        generator: TGenerator,
        generator_state: TGeneratorState,
        cache: VecDeque<Token<T, TData>>,
    }

    /// A wrapping struct for the cache around [`GeneratorTokenQueue`]. Use as the second parameter
    /// in the generator/lexer function
    pub struct GeneratorTokenQueueBuffer<'a, T: PartialEq, TData>(
        &'a mut VecDeque<Token<T, TData>>,
    );

    impl<'a, T: PartialEq, TData> GeneratorTokenQueueBuffer<'a, T, TData> {
        pub fn is_empty(&self) -> bool {
            self.0.is_empty()
        }

        pub fn len(&self) -> usize {
            self.0.len()
        }
    }

    impl<'a, T: PartialEq, TData> TokenSender<T, TData> for GeneratorTokenQueueBuffer<'a, T, TData> {
        fn push(&mut self, token: Token<T, TData>) -> bool {
            self.0.push_back(token);
            true
        }
    }

    impl<T, TData, TGeneratorState, TGenerator>
        GeneratorTokenQueue<T, TData, TGeneratorState, TGenerator>
    where
        T: PartialEq,
        for<'a> TGenerator:
            FnMut(&mut TGeneratorState, &mut GeneratorTokenQueueBuffer<'a, T, TData>) -> (),
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
        T: PartialEq + Debug,
        TData: Debug,
        for<'a> TGenerator:
            FnMut(&mut TGeneratorState, &mut GeneratorTokenQueueBuffer<'a, T, TData>) -> (),
    {
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

        fn peek(&mut self) -> Option<&Token<T, TData>> {
            if self.cache.is_empty() {
                (self.generator)(
                    &mut self.generator_state,
                    &mut GeneratorTokenQueueBuffer(&mut self.cache),
                );
            }
            self.cache.front()
        }

        fn scan(&mut self, mut cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>> {
            let cb = &mut cb;
            let found = scan_cache(&mut self.cache, cb);
            let mut return_next = match found {
                ScanCacheResult::FoundInCache(i) => return self.cache.get(i),
                ScanCacheResult::NotFound => false,
                ScanCacheResult::Found => true,
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

enum ScanCacheResult<T> {
    NotFound,
    FoundInCache(T),
    // Aka pull out next one
    Found,
}

fn scan_cache<T: PartialEq, TData>(
    cache: &mut VecDeque<Token<T, TData>>,
    cb: &mut impl FnMut(&T, &TData) -> bool,
) -> ScanCacheResult<usize> {
    // Scan cache first
    let mut found = None;
    for (i, token) in cache.iter().enumerate() {
        if cb(&token.0, &token.1) {
            found = Some(i);
            break;
        }
    }
    if let Some(i) = found {
        if i < cache.len() {
            return ScanCacheResult::FoundInCache(i);
        }
    }
    if found.is_some() {
        ScanCacheResult::Found
    } else {
        ScanCacheResult::NotFound
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    impl<T: PartialEq + fmt::Debug, TData: PartialEq + fmt::Debug> fmt::Debug for Token<T, TData> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_tuple("Token")
                .field(&self.0)
                .field(&self.0)
                .finish()
        }
    }

    impl<T: PartialEq, TData: PartialEq> PartialEq for Token<T, TData> {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0 && self.1 == other.1
        }
    }

    impl<T: PartialEq, TData: PartialEq> Eq for Token<T, TData> {}

    mod buffered_token_queue {
        use super::{BufferedTokenQueue, Token, TokenReader, TokenSender};

        #[test]
        fn next() {
            let mut stc = BufferedTokenQueue::new();
            stc.push(Token(12, ()));
            stc.push(Token(32, ()));
            stc.push(Token(52, ()));

            assert_eq!(stc.next().unwrap(), Token(12, ()));
            assert_eq!(stc.next().unwrap(), Token(32, ()));
            assert_eq!(stc.next().unwrap(), Token(52, ()));
            assert!(stc.next().is_none());
        }

        #[test]
        fn peek() {
            let mut stc = BufferedTokenQueue::new();
            stc.push(Token(12, ()));

            assert_eq!(stc.peek().unwrap(), &Token(12, ()));
            assert_eq!(stc.next().unwrap(), Token(12, ()));
            assert!(stc.next().is_none());
        }

        #[test]
        fn expect_next() {
            let mut stc = BufferedTokenQueue::new();
            stc.push(Token(12, ()));
            stc.push(Token(24, ()));

            assert_eq!(stc.expect_next(12).unwrap(), ());
            assert!(stc.expect_next(10).is_err());
            assert!(stc.next().is_none());
        }

        #[test]
        fn scan() {
            let mut stc = BufferedTokenQueue::new();
            for val in vec![4, 10, 100, 200] {
                stc.push(Token(val, ()));
            }

            let mut count = 0;
            let x = stc.scan(move |token_val, _| {
                count += token_val;
                count > 100
            });
            assert_eq!(x.unwrap().0, 200);

            let mut count = 0;
            let y = stc.scan(move |token_val, _| {
                count += token_val;
                count > 1000
            });
            assert_eq!(y, None);

            assert_eq!(stc.next().unwrap().0, 4);
            assert_eq!(stc.next().unwrap().0, 10);
            assert_eq!(stc.next().unwrap().0, 100);
            assert_eq!(stc.next().unwrap().0, 200);
            assert!(stc.next().is_none());
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

        fn lexer(state: &mut u8, sender: &mut GeneratorTokenQueueBuffer<u8, ()>) {
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
}
