mod reverse_proxy {
    mod pki;
    mod server;

    mod caddy;
    mod envoy;
    mod haproxy;
    mod nginx;
    mod traefik;

    // CRL revocation checking tests
    mod envoy_crl;
    mod haproxy_crl;
    mod nginx_crl;

    // OCSP revocation checking tests
    mod nginx_ocsp;
    mod ocsp_responder;
}
