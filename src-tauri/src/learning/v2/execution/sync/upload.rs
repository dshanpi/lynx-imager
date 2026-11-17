//! Object-store transport has no access-token argument, proxy, redirect or persisted URL.
use super::*;
use api::{Fault, UploadGrant};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

fn failure(code: &'static str, retryable: bool) -> Fault {
    Fault { code, retryable }
}
fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0 && c == 0)
                && !(a == 192 && b == 88 && c == 99)
                && !(a == 198 && (18..=19).contains(&b))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] <= 0x01ff || s[1] == 0x0db8))
                && s[0] != 0x2002
        }
    }
}
async fn put_bytes(
    client: &reqwest::Client,
    url: reqwest::Url,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<(), Fault> {
    let response = client
        .put(url)
        .header("Content-Type", content_type)
        .header("x-oss-object-acl", "private")
        .body(bytes)
        .send()
        .await
        .map_err(|_| failure("LEARN_UPLOAD_UNCONFIRMED", true))?;
    if response.status().is_redirection() {
        return Err(failure("LEARN_REDIRECT_REJECTED", false));
    }
    if !matches!(response.status().as_u16(), 200 | 201 | 204) {
        return Err(failure("LEARN_UPLOAD_UNCONFIRMED", true));
    }
    // Object-store bodies are neither consumed nor echoed. The API verifies bytes later.
    Ok(())
}
pub(super) async fn put(
    grant: &UploadGrant,
    item: &journal::EvidenceRef,
    bytes: Vec<u8>,
) -> Result<(), Fault> {
    if bytes.len() != item.size
        || hex::encode(Sha256::digest(&bytes)) != item.sha256
        || grant.content_type != item.content_type
    {
        return Err(failure("LEARN_EVIDENCE_CHANGED", false));
    }
    if grant.expires_at <= chrono::Utc::now() {
        return Err(failure("LEARN_UPLOAD_EXPIRED", true));
    }
    let host = grant
        .url
        .domain()
        .ok_or_else(|| failure("LEARN_UPLOAD_ORIGIN_INVALID", false))?;
    if grant.url.scheme() != "https" || grant.url.port_or_known_default() != Some(443) {
        return Err(failure("LEARN_UPLOAD_ORIGIN_INVALID", false));
    }
    let resolved = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((host, 443)),
    )
    .await
    .map_err(|_| failure("LEARN_UPLOAD_DNS_UNAVAILABLE", true))?
    .map_err(|_| failure("LEARN_UPLOAD_DNS_UNAVAILABLE", true))?;
    let addresses: Vec<SocketAddr> = resolved.take(17).collect();
    if addresses.is_empty()
        || addresses.len() > 16
        || addresses
            .iter()
            .any(|a| a.port() != 443 || !public_address(a.ip()))
    {
        return Err(failure("LEARN_UPLOAD_ORIGIN_INVALID", false));
    }
    // Pin the checked addresses, keep certificate verification enabled and bypass
    // ambient proxies so the destination cannot be resolved elsewhere after checking.
    let client = reqwest::Client::builder()
        .no_proxy()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| failure("LEARN_UPLOAD_UNCONFIRMED", true))?;
    if grant.expires_at <= chrono::Utc::now() {
        return Err(failure("LEARN_UPLOAD_EXPIRED", true));
    }
    put_bytes(&client, grant.url.clone(), &grant.content_type, bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_dns_addresses_exclude_private_special_and_mapped_ranges() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.17",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "192.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "::ffff:8.8.8.8",
            "2001:db8::1",
            "2002:7f00:1::",
            "2001::1",
        ] {
            assert!(
                !public_address(address.parse().unwrap()),
                "accepted {address}"
            );
        }
        for address in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111", "2408:4000::1"] {
            assert!(public_address(address.parse().unwrap()));
        }
    }
    #[tokio::test]
    async fn storage_put_contains_exact_bytes_and_private_acl_but_no_login_headers() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    if bytes.len() >= end + 4 + 3 {
                        break;
                    }
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            bytes
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        // Plain HTTP is confined to this unit-test call of the low-level sender.
        put_bytes(
            &client,
            reqwest::Url::parse(&format!("http://{address}/object?Signature=test")).unwrap(),
            "text/plain",
            b"ok\n".to_vec(),
        )
        .await
        .unwrap();
        let wire = worker.join().unwrap();
        let end = wire.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = String::from_utf8_lossy(&wire[..end]).to_lowercase();
        assert!(headers.starts_with("put /object?signature=test "));
        assert!(headers.contains("x-oss-object-acl: private"));
        assert!(headers.contains("content-length: 3"));
        assert!(!headers.contains("authorization:"));
        assert!(!headers.contains("cookie:"));
        assert_eq!(&wire[end + 4..], b"ok\n");
    }
    #[tokio::test]
    async fn expiry_and_wrong_digest_reject_before_any_network_access() {
        let grant=UploadGrant{url:reqwest::Url::parse("https://invalid.invalid/learn/pending/evidence_one/upload_one?Signature=secret-canary").unwrap(),content_type:"text/plain".into(),upload_id:"upload_one".into(),expires_at:(chrono::Utc::now()-chrono::Duration::minutes(1)).fixed_offset()};
        let item = journal::EvidenceRef {
            run_id: "run_one".into(),
            kind: "stdout".into(),
            sha256: hex::encode(Sha256::digest(b"ok\n")),
            size: 3,
            content_type: "text/plain".into(),
            observed_at: "2026-10-18T00:00:00Z".into(),
        };
        assert_eq!(
            put(&grant, &item, b"bad".to_vec()).await.unwrap_err().code,
            "LEARN_EVIDENCE_CHANGED"
        );
        let error = put(&grant, &item, b"ok\n".to_vec()).await.unwrap_err();
        assert_eq!(error.code, "LEARN_UPLOAD_EXPIRED");
        assert!(!format!("{error:?}").contains("secret-canary"));
    }
}
