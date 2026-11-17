use url::Url;

// Local-only status page URL served from bundle Resources/status/
pub fn url() -> Url {
    Url::parse("qcliresource://localhost/status/").expect("valid local status url")
}