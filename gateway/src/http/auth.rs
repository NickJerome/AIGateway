//! Authorization header decoding belongs to the HTTP boundary.
pub(super) fn bearer(headers: &hyper::HeaderMap) -> Option<&str> {
    let mut values = headers.get_all("authorization").iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && token.len() <= 256
        && !token.bytes().any(|b| b.is_ascii_whitespace()))
    .then_some(token)
}
