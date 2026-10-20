use std::net::Ipv4Addr;

use serde_json::{Value, json};

use crate::{CoreError, CoreResult, Settings};

pub struct ProxyConfigOptions {
    pub admin_listen: String,
    pub listen_addresses: Vec<Ipv4Addr>,
    pub http_port: u16,
    pub https_port: u16,
    pub setup_port: u16,
}

impl ProxyConfigOptions {
    pub fn for_settings(settings: &Settings) -> CoreResult<Self> {
        let lan = settings.lan_address.parse().map_err(|_| CoreError::Validation { message: "A valid LAN IPv4 address is required".into() })?;
        Ok(Self { admin_listen: "127.0.0.1:2020".into(), listen_addresses: vec![Ipv4Addr::LOCALHOST, lan], http_port: 80, https_port: 443, setup_port: 4781 })
    }
}

pub fn build_proxy_config(settings: &Settings, options: &ProxyConfigOptions) -> CoreResult<Value> {
    let enabled: Vec<_> = settings.mappings.iter().filter(|mapping| mapping.enabled).collect();
    let mut http_routes = vec![proxy_route(&["setup.own", &settings.lan_address], options.setup_port)];
    http_routes.extend(enabled.iter().map(|mapping| if mapping.tls {
        json!({"match":[{"host":[mapping.hostname]}],"handle":[{"handler":"static_response","headers":{"Location":[format!("https://{}{{http.request.uri}}", mapping.hostname)]},"status_code":308}],"terminal":true})
    } else { proxy_route(&[&mapping.hostname], mapping.upstream_port) }));
    http_routes.push(not_found());
    let tls: Vec<_> = enabled.iter().filter(|mapping| mapping.tls).collect();
    let mut https_routes: Vec<_> = tls.iter().map(|mapping| proxy_route(&[&mapping.hostname], mapping.upstream_port)).collect();
    https_routes.push(not_found());
    Ok(json!({
        "admin":{"listen":options.admin_listen},
        "apps":{"http":{"http_port":options.http_port,"https_port":options.https_port,"servers":{
            "http":{"listen":listeners(options, options.http_port),"routes":http_routes},
            "https":{"listen":listeners(options, options.https_port),"routes":https_routes,"tls_connection_policies":[{}],"automatic_https":{"disable_redirects":true}}
        }},"tls":{"automation":{"policies":tls.iter().map(|mapping| json!({"subjects":[mapping.hostname],"issuers":[{"module":"internal"}]})).collect::<Vec<_>>()}}}
    }))
}

fn listeners(options: &ProxyConfigOptions, port: u16) -> Vec<String> { options.listen_addresses.iter().map(|address| format!("{address}:{port}")).collect() }
fn proxy_route(hosts: &[&str], port: u16) -> Value { json!({"match":[{"host":hosts}],"handle":[{"handler":"reverse_proxy","upstreams":[{"dial":format!("127.0.0.1:{port}")}]}],"terminal":true}) }
fn not_found() -> Value { json!({"handle":[{"handler":"static_response","body":"Unknown .own domain","status_code":404}],"terminal":true}) }
