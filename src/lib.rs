use std::{
    cell::UnsafeCell,
    collections::VecDeque,
    sync::mpsc::{sync_channel, Receiver, RecvError, SyncSender},
};

#[macro_export]
macro_rules! expect_next {
    ($reader:expr, $pattern:pat) => {{
        let next = $reader.next();
        if let Some(token) = next {
            if matches!(token.0, $pattern) {
                Ok(token.1)
            } else {
                Err(Some((stringify!($pattern), token)))
            }
        } else {
            Err(None)
        }
    }};
}

/// Defines a Token of type T and with a Position
pub struct Token<T: PartialEq, TData>(pub T, pub TData);

/// Trait for a reader which returns tokens over a current sequence
pub trait TokenReader<T: PartialEq, TData> {
    /// Returns reference to next token but does not advance iterator forward
    fn peek(&self) -> Option<&Token<T, TData>>;
    /// Returns token and advances forward
    fn next(&mut self) -> Option<Token<T, TData>>;

    /// Runs the closure over the upcoming tokens. Passes the value behind the Token to the closure.
    /// Will stop and return a reference to the next Token from when the closure returns true.
    /// Returns None if reader finishes before closure returns true. Does not advance the reader.
    ///
    /// Used for lookahead and then doing branching based on return value during parsing
    fn scan(&self, f: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>>;

    /// Tests that next token matches an expected type. Will return error if does not
    /// match. The ok value contains the data of the valid token.
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
    /// Appends new Token
    fn push(&mut self, token: Token<T, TData>);
}

/// A synchronous "channel" which can be used as a sender and reader. Will
/// buffer all tokens into a `VecDeque` before reading
pub struct StaticTokenChannel<T: PartialEq, TData> {
    tokens: VecDeque<Token<T, TData>>,
}

impl<T: PartialEq, TData> StaticTokenChannel<T, TData> {
    pub fn new() -> Self {
        StaticTokenChannel {
            tokens: VecDeque::new(),
        }
    }
}

impl<T: PartialEq, TData> TokenSender<T, TData> for StaticTokenChannel<T, TData> {
    fn push(&mut self, token: Token<T, TData>) {
        self.tokens.push_back(token)
    }
}

impl<T: PartialEq, TData> TokenReader<T, TData> for StaticTokenChannel<T, TData> {
    fn peek(&self) -> Option<&Token<T, TData>> {
        self.tokens.front()
    }

    fn next(&mut self) -> Option<Token<T, TData>> {
        self.tokens.pop_front()
    }

    fn scan(&self, mut cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>> {
        let mut iter = self.tokens.iter().peekable();
        while let Some(token) = iter.next() {
            if cb(&token.0, &token.1) {
                return iter.peek().map(|v| *v);
            }
        }
        None
    }
}

#[allow(non_snake_case)]
pub mod StreamedTokenChannel {
    const STREAMED_CHANNEL_BUFFER_SIZE: usize = 20;

    use super::*;

    pub struct StreamedTokenSender<T: PartialEq, TData>(SyncSender<Token<T, TData>>);
    pub struct StreamedTokenReader<T: PartialEq, TData> {
        receiver: Receiver<Token<T, TData>>,
        cache: UnsafeCell<VecDeque<Token<T, TData>>>,
    }

    impl<T: PartialEq, TData> TokenSender<T, TData> for StreamedTokenSender<T, TData> {
        fn push(&mut self, token: Token<T, TData>) {
            self.0.send(token).unwrap();
        }
    }

    impl<T: PartialEq, TData> TokenReader<T, TData> for StreamedTokenReader<T, TData> {
        fn peek(&self) -> Option<&Token<T, TData>> {
            // SAFETY: mutable reference needed to added to cache. RefCell returns Ref<T, TData> not &T.
            // no methods on StreamedTokenReader return &mut to values in the cache
            let cache = unsafe { &mut *self.cache.get() };
            if cache.is_empty() {
                match self.receiver.recv() {
                    Ok(val) => cache.push_back(val),
                    // Err is reader has dropped e.g. no more tokens
                    Err(RecvError) => {
                        return None;
                    }
                }
            }
            cache.front()
        }

        fn next(&mut self) -> Option<Token<T, TData>> {
            // SAFETY: safe to get mutable reference for this function as have mutable self
            let cache = unsafe { &mut *self.cache.get() };
            if !cache.is_empty() {
                return cache.pop_front();
            }
            self.receiver.recv().ok()
        }

        fn scan(&self, mut cb: impl FnMut(&T, &TData) -> bool) -> Option<&Token<T, TData>> {
            let mut found = false;
            for token in unsafe { &*self.cache.get() }.iter() {
                if found {
                    return Some(token);
                }
                if cb(&token.0, &token.1) {
                    found = true;
                }
            }
            // SAFETY: mutable reference needed to added to cache. RefCell returns Ref<T> not &T.
            // no methods on StreamedTokenReader return &mut to values in the cache
            let cache = unsafe { &mut *self.cache.get() };
            loop {
                match self.receiver.recv() {
                    Ok(val) => {
                        if found {
                            cache.push_back(val);
                            return cache.back();
                        }
                        if cb(&val.0, &val.1) {
                            found = true;
                        }
                        cache.push_back(val);
                    }
                    // Err is reader has dropped e.g. no more tokens
                    Err(RecvError) => {
                        return None;
                    }
                }
            }
        }
    }

    /// Will return a `TokenSender` and `TokenReader` for use when lexing and parsing in separate threads
    /// Unlike `StaticTokenChannel` it does not buffer all the tokens before parsing can begin
    pub fn new<T: PartialEq, TData>(
    ) -> (StreamedTokenSender<T, TData>, StreamedTokenReader<T, TData>) {
        let (sender, receiver) = sync_channel::<Token<T, TData>>(STREAMED_CHANNEL_BUFFER_SIZE);
        (
            StreamedTokenSender(sender),
            StreamedTokenReader {
                receiver,
                cache: UnsafeCell::new(VecDeque::new()),
            },
        )
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

    mod static_token_channel {
        use super::{StaticTokenChannel, TokenReader, TokenSender, Token};

        #[test]
        fn next() {
            let mut stc = StaticTokenChannel::new();
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
            let mut stc = StaticTokenChannel::new();
            stc.push(Token(12, ()));

            assert_eq!(stc.peek().unwrap(), &Token(12, ()));
            assert_eq!(stc.next().unwrap(), Token(12, ()));
            assert!(stc.next().is_none());
        }

        #[test]
        fn expect_next() {
            let mut stc = StaticTokenChannel::new();
            stc.push(Token(12, ()));
            stc.push(Token(24, ()));

            assert_eq!(stc.expect_next(12).unwrap(), ());
            assert!(stc.expect_next(10).is_err());
            assert!(stc.next().is_none());
        }

        #[test]
        fn scan() {
            let mut stc = StaticTokenChannel::new();
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

    mod streamed_token_channel {
        use super::{StreamedTokenChannel, TokenReader, TokenSender, Token};

        #[test]
        fn next() {
            let (mut sender, mut reader) = StreamedTokenChannel::new();
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
            let (mut sender, mut reader) = StreamedTokenChannel::new();
            std::thread::spawn(move || {
                sender.push(Token(12, ()));
            });

            assert_eq!(reader.peek().unwrap(), &Token(12, ()));
            assert_eq!(reader.next().unwrap(), Token(12, ()));
            assert!(reader.next().is_none());
        }

        #[test]
        fn expect_next() {
            let (mut sender, mut reader) = StreamedTokenChannel::new();
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
            let (mut sender, mut reader) = StreamedTokenChannel::new();
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

    #[test]
    fn expect_next_macro() {
        let mut stc = StaticTokenChannel::new();
        stc.push(Token(12, ()));
        stc.push(Token(32, ()));

        assert!(dbg!(expect_next!(stc, 12)).is_ok());
        assert!(dbg!(expect_next!(stc, 23)).is_err());
    }
}
