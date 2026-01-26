mod common;
use common::*;

#[test]
fn tcp_connect_and_close() {
    let code = r#"
needs std.net
let sock = net.connect("www.google.com", 80)
net.close(sock)
42
"#;
    // Requires network capability - either succeeds or capability denied
    let result = run_aelys_result(code);
    match result {
        Ok(v) => assert_eq!(v.as_int(), Some(42)),
        Err(e) => assert!(e.contains("capability")),
    }
}

#[test]
fn net_invalid_port_negative() {
    let code = r#"
needs std.net
net.connect("localhost", -1)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("port") || err.contains("invalid") || err.contains("capability"));
}

#[test]
fn invalid_port_too_large() {
    let code = r#"
needs std.net
net.connect("localhost", 99999)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("port") || err.contains("capability"));
}

#[test]
fn connect_timeout_unreachable() {
    // 10.255.255.1 is typically unreachable
    let code = r#"
needs std.net
net.connect("10.255.255.1", 9999)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("failed") || err.contains("connection") || err.contains("timeout") || err.contains("capability"));
}

#[test]
fn invalid_socket_handle() {
    let code = r#"
needs std.net
net.send(999, "test")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn recv_on_invalid_handle() {
    let code = r#"
needs std.net
net.recv(123)
"#;
    let err = run_aelys_err(code);
    assert!(err.to_lowercase().contains("invalid") || err.to_lowercase().contains("handle") || err.contains("capability"));
}

#[test]
fn recv_bytes_negative_max() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.recv_bytes(s, -5)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("negative") || err.contains("capability"));
}

#[test]
fn recv_bytes_exceeds_max_buffer() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.recv_bytes(s, 17000000)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("max") || err.contains("buffer") || err.contains("exceeds") || err.contains("capability"));
}

#[test]
fn http_get_request() {
    let code = r#"
needs std.net
let sock = net.connect("www.example.com", 80)
net.send(sock, "GET / HTTP/1.0\r\nHost: www.example.com\r\n\r\n")
let response = net.recv(sock)
net.close(sock)
42
"#;
    // Requires network capability
    let result = run_aelys_result(code);
    match result {
        Ok(v) => assert_eq!(v.as_int(), Some(42)),
        Err(e) => assert!(e.contains("capability")),
    }
}

#[test]
fn set_timeout_invalid_handle() {
    let code = r#"
needs std.net
net.set_timeout(777, 1000)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn set_timeout_negative_ms() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.set_timeout(s, -100)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("negative") || err.contains("capability"));
}

#[test]
fn set_nodelay_works() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.set_nodelay(s, true)
net.close(s)
1
"#;
    // Requires network capability
    let result = run_aelys_result(code);
    match result {
        Ok(v) => assert_eq!(v.as_int(), Some(1)),
        Err(e) => assert!(e.contains("capability")),
    }
}

#[test]
fn shutdown_modes() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.shutdown(s, "both")
net.close(s)
1
"#;
    // Requires network capability
    let result = run_aelys_result(code);
    match result {
        Ok(v) => assert_eq!(v.as_int(), Some(1)),
        Err(e) => assert!(e.contains("capability")),
    }
}

#[test]
fn shutdown_invalid_mode() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.shutdown(s, "invalid")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("mode") || err.contains("capability"));
}

#[test]
fn local_and_peer_addr() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
let local = net.local_addr(s)
let peer = net.peer_addr(s)
net.close(s)
1
"#;
    // Requires network capability
    let result = run_aelys_result(code);
    match result {
        Ok(v) => assert_eq!(v.as_int(), Some(1)),
        Err(e) => assert!(e.contains("capability")),
    }
}

#[test]
fn listen_invalid_port() {
    let code = r#"
needs std.net
net.listen("0.0.0.0", -5)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("port") || err.contains("capability"));
}

#[test]
fn close_invalid_handle() {
    let code = r#"
needs std.net
net.close(456)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn recv_line_basic() {
    // Hard to test without a real server
    // Just test invalid handle
    let code = r#"
needs std.net
net.recv_line(999)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn double_close() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.close(s)
net.close(s)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn send_after_close() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.close(s)
net.send(s, "data")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn connect_dns_failure() {
    let code = r#"
needs std.net
net.connect("this.domain.does.not.exist.anywhere.invalid", 80)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("resolve") || err.contains("failed") || err.contains("connection") || err.contains("capability"));
}

#[test]
fn local_addr_after_close() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.close(s)
net.local_addr(s)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn peer_addr_invalid() {
    let code = r#"
needs std.net
net.peer_addr(12345)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn multiple_connections() {
    let code = r#"
needs std.net
let s1 = net.connect("www.google.com", 80)
let s2 = net.connect("www.example.com", 80)
net.close(s1)
net.close(s2)
42
"#;
    // Requires network capability
    let result = run_aelys_result(code);
    match result {
        Ok(v) => assert_eq!(v.as_int(), Some(42)),
        Err(e) => assert!(e.contains("capability")),
    }
}

#[test]
fn zero_timeout_disables() {
    let code = r#"
needs std.net
let s = net.connect("www.google.com", 80)
net.set_timeout(s, 0)
net.close(s)
1
"#;
    // Requires network capability
    let result = run_aelys_result(code);
    match result {
        Ok(v) => assert_eq!(v.as_int(), Some(1)),
        Err(e) => assert!(e.contains("capability")),
    }
}
