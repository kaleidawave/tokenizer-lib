use std::{
    collections::VecDeque,
    fmt,
    sync::mpsc::{sync_channel, Receiver, RecvError, SyncSender},
};

/// Represents a start and end of something in a sequence
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span(pub usize, pub usize);

/// Defines a Token of type T and with a Position
#[derive(PartialEq, Eq)]
pub struct Token<T>(pub T, pub Span);

impl<T: fmt::Debug> fmt::Debug for Token<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} {}:{}", self.0, self.1 .0, self.1 .1)
    }
}

/// A error struct with a reason and position
pub struct ParseError {
    pub reason: String,
    pub position: Option<Span>,
}

impl fmt::Debug for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ParseError: {}{}",
            self.reason,
            if let Some(pos) = &self.position {
                format!(" {:?}", pos)
            } else {
                "".to_owned()
            }
        )
    }
}

/// Trait for a reader which returns tokens over a current sequence
pub trait TokenReader<T>
where
    T: PartialEq + fmt::Debug,
{
    /// Returns reference to next token but does not advance iterator forward
    fn peek(&mut self) -> Option<&Token<T>>;
    /// Returns token and advances forward
    fn next(&mut self) -> Option<Token<T>>;

    /// Tests that next token matches an expected type. Will return `ParseError` if does not
    /// match. Else it will return the position of the correctly matching token
    fn expect_next(&mut self, expected_type: T) -> Result<Span, ParseError> {
        match self.next() {
            Some(Token(token_type, position)) => {
                if token_type != expected_type {
                    Err(ParseError {
                        reason: format!("Expected {:?}, received {:?}", expected_type, token_type),
                        position: Some(position),
                    })
                } else {
                    Ok(position)
                }
            }
            None => Err(ParseError {
                reason: format!("Expected {:?} but reached end of source", expected_type),
                position: None,
            }),
        }
    }
}

/// Trait for a sender that can append a token to a sequence
pub trait TokenSender<T> {
    /// Appends new Token
    fn push(&mut self, token: Token<T>);
}

/// A synchronous "channel" which can be used as a sender and reader. Will
/// buffer all tokens into a `VecDeque` before reading
pub struct StaticTokenChannel<T> {
    tokens: VecDeque<Token<T>>,
}

impl<T> StaticTokenChannel<T> {
    pub fn new() -> Self {
        StaticTokenChannel {
            tokens: VecDeque::new(),
        }
    }
}

impl<T> TokenSender<T> for StaticTokenChannel<T> {
    fn push(&mut self, token: Token<T>) {
        self.tokens.push_back(token)
    }
}

impl<T: PartialEq + fmt::Debug> TokenReader<T> for StaticTokenChannel<T> {
    fn peek(&mut self) -> Option<&Token<T>> {
        self.tokens.front()
    }

    fn next(&mut self) -> Option<Token<T>> {
        self.tokens.pop_front()
    }
}

pub struct StreamedTokenSender<T>(SyncSender<Token<T>>);
pub struct StreamedTokenReader<T> {
    receiver: Receiver<Token<T>>,
    cache: VecDeque<Token<T>>,
}

impl<T> TokenSender<T> for StreamedTokenSender<T> {
    fn push(&mut self, token: Token<T>) {
        self.0.send(token).unwrap();
    }
}

/// Will return a `TokenSender` and `TokenReader` for use when lexing and parsing in separate threads
/// Unlike `StaticTokenChannel` it does not buffer all the tokens before parsing can begin
pub fn get_streamed_token_channel<T>() -> (StreamedTokenSender<T>, StreamedTokenReader<T>) {
    let (sender, receiver) = sync_channel::<Token<T>>(20);
    (
        StreamedTokenSender(sender),
        StreamedTokenReader {
            receiver,
            cache: VecDeque::new(),
        },
    )
}

impl<T: PartialEq + fmt::Debug> TokenReader<T> for StreamedTokenReader<T> {
    fn peek(&mut self) -> Option<&Token<T>> {
        if self.cache.is_empty() {
            match self.receiver.recv() {
                Ok(val) => self.cache.push_back(val),
                // Err is reader has dropped e.g. no more tokens
                Err(RecvError) => {
                    return None;
                }
            }
        }
        self.cache.front()
    }

    fn next(&mut self) -> Option<Token<T>> {
        if !self.cache.is_empty() {
            return self.cache.pop_front();
        }
        self.receiver.recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_token_channel() {
        let mut stc = StaticTokenChannel::new();
        stc.push(Token(12, Span(0, 2)));
        stc.push(Token(32, Span(2, 4)));
        stc.push(Token(52, Span(4, 8)));

        assert_eq!(stc.next().unwrap(), Token(12, Span(0, 2)));
        assert_eq!(stc.next().unwrap(), Token(32, Span(2, 4)));
        assert_eq!(stc.next().unwrap(), Token(52, Span(4, 8)));
        assert_eq!(stc.next(), None);
    }

    #[test]
    fn static_token_channel_peek() {
        let mut stc = StaticTokenChannel::new();
        stc.push(Token(12, Span(0, 2)));

        assert_eq!(stc.peek().unwrap(), &Token(12, Span(0, 2)));
        assert_eq!(stc.next().unwrap(), Token(12, Span(0, 2)));
        assert_eq!(stc.next(), None);
    }

    #[test]
    fn static_token_channel_expect_next() {
        let mut stc = StaticTokenChannel::new();
        stc.push(Token(12, Span(0, 2)));
        stc.push(Token(24, Span(2, 4)));

        assert_eq!(stc.expect_next(12).unwrap(), Span(0, 2));
        let err = stc.expect_next(10).unwrap_err();
        assert_eq!(err.position, Some(Span(2, 4)));
        assert_eq!(err.reason, "Expected 10, received 24".to_owned());
        assert_eq!(stc.next(), None);
    }

    #[test]
    fn streamed_token_channel() {
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
    }

    #[test]
    fn streamed_token_channel_peek() {
        let (mut sender, mut reader) = get_streamed_token_channel();
        std::thread::spawn(move || {
            sender.push(Token(12, Span(0, 2)));
        });

        assert_eq!(reader.peek().unwrap(), &Token(12, Span(0, 2)));
        assert_eq!(reader.next().unwrap(), Token(12, Span(0, 2)));
        assert_eq!(reader.next(), None);
    }

    #[test]
    fn streamed_token_channel_expect_next() {
        let (mut sender, mut reader) = get_streamed_token_channel();
        std::thread::spawn(move || {
            sender.push(Token(12, Span(0, 2)));
            sender.push(Token(24, Span(2, 4)));
        });

        assert_eq!(reader.expect_next(12).unwrap(), Span(0, 2));
        let err = reader.expect_next(10).unwrap_err();
        assert_eq!(err.position, Some(Span(2, 4)));
        assert_eq!(err.reason, "Expected 10, received 24".to_owned());
        assert_eq!(reader.next(), None);
    }
}
