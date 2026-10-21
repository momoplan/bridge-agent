pub(crate) fn parse_relay_websocket_base(raw: &str) -> Result<Url> {
    let url = Url::parse(raw.trim()).context("relay.url must be a valid URL")?;
    ensure_relay_websocket_scheme(&url)?;
    Ok(url)
}

pub(crate) fn ensure_relay_websocket_scheme(url: &Url) -> Result<()> {
    match url.scheme() {
        "wss" => Ok(()),
        "ws" if relay_host_allows_plaintext(url) => Ok(()),
        "ws" => {
            bail!("relay.url must use wss://; plaintext ws:// is only allowed for localhost")
        }
        other => bail!("relay.url scheme `{other}` is not supported; use wss://"),
    }
}

fn relay_host_allows_plaintext(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host
            .trim_end_matches('.')
            .eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}
