use std::str::Utf8Error;

use nom::{
    bytes::complete::{is_not, tag, tag_no_case, take_until1, take_while, take_while1},
    character::is_space,
    combinator::{map, map_res, not},
    IResult,
};
use thiserror::Error;

#[inline]
fn challenge(input: &[u8]) -> IResult<&[u8], Challenge> {
    // Skip whitespace.
    let (input, _) = take_while(is_space)(input)?;

    let (input, scheme) = scheme(input)?;

    match scheme {
        Scheme::Basic => {
            let (input, params) = scheme_params_basic(input)?;
            Ok((input, Challenge::Basic(params)))
        }
        _ => Ok((input, Challenge::Unsupported(scheme))),
    }
}

fn quoted_string()

fn realm(input: &[u8]) -> IResult<&[u8], &[u8]> {
    let (input, _) = tag_no_case("realm=")(input)?;

    // Skip whitespace, just in case.
    let (input, _) = take_while(is_space)(input)?;

    let name = todo!();

    Ok((input, name))
}

fn scheme_params_basic(input: &[u8]) -> IResult<&[u8], BasicChallenge> {
    todo!()
}

#[inline(always)]
fn not_whitespace(input: &[u8]) -> IResult<&[u8], &[u8]> {
    is_not(&b" \t\r\n"[..])(input)
}

/// Parses a scheme.
#[inline]
fn scheme(input: &[u8]) -> IResult<&[u8], Scheme> {
    (map_res(not_whitespace, |bytes| Scheme::from_bytestr(bytes)))(input)
}

#[cfg(test)]
mod tests {
    use super::{scheme, Scheme};

    #[test]
    fn parses_scheme() {
        assert_eq!(Ok((&b"  "[..], Scheme::Basic)), scheme(b"bAsIc  "));
        assert_eq!(Ok((&b""[..], Scheme::Basic)), scheme(b"BASIC"));
        assert!(scheme(b"invalid").is_err());
    }
}

#[derive(Debug)]
pub(crate) enum Challenge {
    Basic(BasicChallenge),
    Unsupported(Scheme),
}

#[derive(Debug, Default)] // TODO: Use `sec` here instead.
struct BasicChallenge {
    realm: Option<String>,
    charset: Option<String>,
}

impl Challenge {
    fn from_bytestr(input: &[u8]) -> Result<(Challenge, &[u8]), Error> {
        let mut parts = input
            .split(u8::is_ascii_whitespace)
            .filter(|sl| !sl.is_empty());

        let scheme = Scheme::from_bytestr(parts.next().ok_or(Error::UnexpectedEnd)?)
            .map_err(Error::InvalidScheme)?;

        let challenge = match scheme {
            Scheme::Basic => {
                let mut basic = BasicChallenge::default();

                for part in parts {
                    if part.starts_with(b"realm=") {}
                }

                Challenge::Basic(basic)
            }

            _ => Challenge::Unsupported(scheme),
        };

        todo!()
    }
}

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("invalid utf8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("unexpected end parsing challenge")]
    UnexpectedEnd,
    #[error("invalid scheme")]
    InvalidScheme(#[source] InvalidScheme),
    #[error("unsupported scheme")]
    UnsupportedScheme,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Scheme {
    Basic,
    Bearer,
    Digest,
    Hoba,
    Mutual,
    Negotiate,
    Vapid,
    Scram,
    Aws4HmacSha256,
}

#[derive(Copy, Clone, Debug, Error)]
#[error("invalid authentication scheme")]

struct InvalidScheme;

// from unstable stdlib `trim_ascii_start`
pub const fn trim_ascii_start(mut bytes: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = bytes {
        if first.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

// from unstable stdlib `trim_ascii_end`
pub const fn trim_ascii_end(mut bytes: &[u8]) -> &[u8] {
    while let [rest @ .., last] = bytes {
        if last.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

impl Scheme {
    #[inline]
    fn from_bytestr(mut s: &[u8]) -> Result<Self, InvalidScheme> {
        let lowercased = trim_ascii_start(trim_ascii_end(s)).to_ascii_lowercase();

        match &lowercased[..] {
            b"basic" => Ok(Scheme::Basic),
            b"bearer" => Ok(Scheme::Bearer),
            b"digest" => Ok(Scheme::Digest),
            b"hoba" => Ok(Scheme::Hoba),
            b"mutual" => Ok(Scheme::Mutual),
            b"negotiate" => Ok(Scheme::Negotiate),
            b"vapid" => Ok(Scheme::Vapid),
            b"scram" => Ok(Scheme::Scram),
            b"aws-hmac-sha256" => Ok(Scheme::Aws4HmacSha256),
            _ => Err(InvalidScheme),
        }
    }
}

// impl FromStr for Scheme {
//     type Err = InvalidScheme;

//     #[inline(always)]
//     fn from_str(s: &str) -> Result<Self, Self::Err> {
//         Scheme::from_bytestr(s.as_bytes())
//     }
// }
