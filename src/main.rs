use async_trait::async_trait;
use clap::Parser;
use log::info;
use pingora::http::ResponseHeader;
use prometheus::register_int_counter;

use pingora::server::configuration::Opt;
use pingora::server::Server;
use pingora::upstreams::peer::HttpPeer;
use pingora::Result;
use pingora::prelude::*;

fn check_login(req: &RequestHeader) -> bool {
    // implement you logic check logic here
    req.headers.get("Authorization").map(|v| v.as_bytes()) == Some(b"password")
}

pub struct MyGateway {
    req_metric: prometheus::IntCounter,
}

#[async_trait]
impl ProxyHttp for MyGateway {
    type CTX = ();
    fn new_ctx(&self) -> Self::CTX {}

    async fn request_filter(&self, session: &mut Session, _ctx: &mut Self::CTX) -> Result<bool> {
        if session.req_header().uri.path().starts_with("/login")
            && !check_login(session.req_header())
        {
            let _ = session.respond_error(403).await;
            // true: early return as the response is already written
            return Ok(true);
        }
        Ok(false)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let addr = if session.req_header().uri.path().starts_with("/family") {
            ("1.0.0.1", 443)
        } else {
            ("1.1.1.1", 443)
        };

        info!("connecting to {addr:?}");

        let peer = Box::new(HttpPeer::new(addr, true, "one.one.one.one".to_string()));
        Ok(peer)
    }

    async fn response_filter(
        &self,
        _session: &mut Session,
        upstream_response: &mut ResponseHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()>
    where
        Self::CTX: Send + Sync,
    {
        // replace existing header if any
        upstream_response
            .insert_header("Server", "MyGateway")
            .unwrap();
        // because we don't support h3
        upstream_response.remove_header("alt-svc");

        Ok(())
    }

    async fn logging(
        &self,
        session: &mut Session,
        _e: Option<&pingora::Error>,
        ctx: &mut Self::CTX,
    ) {
        let response_code = session
            .response_written()
            .map_or(0, |resp| resp.status.as_u16());
        info!(
            "{} response code: {response_code}",
            self.request_summary(session, ctx)
        );

        self.req_metric.inc();
    }
}

// RUST_LOG=INFO cargo run --example load_balancer
// curl 127.0.0.1:6191 -H "Host: one.one.one.one"
// curl 127.0.0.1:6190/family/ -H "Host: one.one.one.one"
// curl 127.0.0.1:6191/login/ -H "Host: one.one.one.one" -I -H "Authorization: password"
// curl 127.0.0.1:6191/login/ -H "Host: one.one.one.one" -I -H "Authorization: bad"
// For metrics
// curl 127.0.0.1:6192/
fn main() {
    env_logger::init();
    // read command line arguments
    let opt = Opt::parse();
    let mut my_server = Server::new(Some(opt)).unwrap();
    my_server.bootstrap();

    let mut my_proxy = http_proxy_service(
        &my_server.configuration,
        MyGateway {
            req_metric: register_int_counter!("req_counter", "Number of requests").unwrap(),
        },
    );
    my_proxy.add_tcp("0.0.0.0:6191");
    my_server.add_service(my_proxy);

    my_server.run_forever();
}
