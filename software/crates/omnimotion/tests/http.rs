use omnimotion::net::http::{
    constant_time_equal, head_end, normalized_path, parse_head, Head, HeadError, PATH_LIMIT,
};

#[test]
fn a_head_ends_at_the_blank_line() {
    assert_eq!(head_end(b"GET / HTTP/1.1\r\nHost: x\r\n"), None);
    let request = b"GET / HTTP/1.1\r\nHost: x\r\n\r\nbody";
    assert_eq!(head_end(request), Some(request.len() - 4));
}

#[test]
fn heads_give_the_route_length_and_key() {
    let head = parse_head(
        b"POST /weave?x=1 HTTP/1.1\r\ncontent-length: 12\r\nX-API-Key:  secret \r\n\r\n",
        b"secret",
    )
    .unwrap();
    assert_eq!(
        head,
        Head {
            method: "POST",
            path: "/weave",
            content_length: Some(12),
            authorized: true,
        }
    );
    assert_eq!(head.body_length(), Ok(12));
    let get = parse_head(
        b"GET /status HTTP/1.0\r\nX-API-Key: wrong\r\n\r\n",
        b"secret",
    )
    .unwrap();
    assert!(!get.authorized);
    assert_eq!(get.body_length(), Ok(0));
    let post = parse_head(b"POST /stop HTTP/1.1\r\n\r\n", b"secret").unwrap();
    assert_eq!(post.body_length(), Err(HeadError::LengthRequired));
}

#[test]
fn an_empty_token_authorizes_nothing() {
    let head = parse_head(b"GET /status HTTP/1.1\r\nX-API-Key: \r\n\r\n", b"").unwrap();
    assert!(!head.authorized);
}

#[test]
fn chunked_bodies_need_a_length() {
    assert_eq!(
        parse_head(
            b"POST /weave HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"k"
        ),
        Err(HeadError::LengthRequired)
    );
}

#[test]
fn malformed_heads_are_refused() {
    for head in [
        &b"GET /status\r\n\r\n"[..],
        b"GET /status HTTP/2\r\n\r\n",
        b"GET /status HTTP/1.1 extra\r\n\r\n",
        b"GET /../x HTTP/1.1\r\n\r\n",
        b"LONGMETHOD / HTTP/1.1\r\n\r\n",
        b"POST /weave HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n",
        b"POST /weave HTTP/1.1\r\nContent-Length: x\r\n\r\n",
        b"GET / HTTP/1.1\r\nno colon\r\n\r\n",
        b"GET /\xff HTTP/1.1\r\n\r\n",
    ] {
        assert_eq!(
            parse_head(head, b"k"),
            Err(HeadError::Malformed),
            "{head:?}"
        );
    }
    let long = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(PATH_LIMIT));
    assert_eq!(parse_head(long.as_bytes(), b"k"), Err(HeadError::Malformed));
}

#[test]
fn paths_are_canonical() {
    assert_eq!(normalized_path("/?cache=1"), Some("/"));
    assert_eq!(normalized_path("/script.js"), Some("/script.js"));
    assert_eq!(normalized_path("/../secret"), None);
    assert_eq!(normalized_path("/%2e%2e/secret"), None);
    assert_eq!(normalized_path("//script.js"), None);
}

#[test]
fn authentication_compares_content_and_length() {
    assert!(constant_time_equal(b"secret", b"secret"));
    assert!(!constant_time_equal(b"secret", b"secrex"));
    assert!(!constant_time_equal(b"secret", b"secret-long"));
}
