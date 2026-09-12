use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use loop_protocol::wire::jobs::v1::job_service_client::JobServiceClient;
use sha2::{Digest, Sha256};
use tonic::transport::{
    Certificate, Channel, ClientTlsConfig, Endpoint, Identity, ServerTlsConfig,
};

pub(super) struct Credentials {
    root: PathBuf,
}

impl Credentials {
    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    pub(super) fn new(parent: &Path) -> Self {
        let root = parent.join("tls");
        std::fs::create_dir(&root).unwrap();
        openssl(
            &root,
            &[
                "req",
                "-x509",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:prime256v1",
                "-nodes",
                "-keyout",
                "ca.key",
                "-out",
                "ca.pem",
                "-subj",
                "/CN=Loop-test-CA",
                "-days",
                "1",
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
            ],
        );
        for name in ["server", "client", "unknown"] {
            openssl(
                &root,
                &[
                    "req",
                    "-new",
                    "-newkey",
                    "ec",
                    "-pkeyopt",
                    "ec_paramgen_curve:prime256v1",
                    "-nodes",
                    "-keyout",
                    &format!("{name}.key"),
                    "-out",
                    &format!("{name}.csr"),
                    "-subj",
                    "/CN=localhost",
                    "-addext",
                    "subjectAltName=DNS:localhost",
                ],
            );
            openssl(
                &root,
                &[
                    "x509",
                    "-req",
                    "-in",
                    &format!("{name}.csr"),
                    "-CA",
                    "ca.pem",
                    "-CAkey",
                    "ca.key",
                    "-CAcreateserial",
                    "-out",
                    &format!("{name}.pem"),
                    "-days",
                    "1",
                    "-copy_extensions",
                    "copy",
                ],
            );
        }
        openssl(
            &root,
            &[
                "x509",
                "-in",
                "client.pem",
                "-outform",
                "DER",
                "-out",
                "client.der",
            ],
        );
        Self { root }
    }

    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.root.join(name)).unwrap()
    }
    pub(super) fn client_digest(&self) -> String {
        format!("sha256:{:x}", Sha256::digest(self.read("client.der")))
    }
    pub(super) fn server(&self) -> ServerTlsConfig {
        ServerTlsConfig::new()
            .identity(Identity::from_pem(
                self.read("server.pem"),
                self.read("server.key"),
            ))
            .client_ca_root(Certificate::from_pem(self.read("ca.pem")))
    }
    pub(super) async fn client(
        &self,
        address: std::net::SocketAddr,
        authenticated: bool,
    ) -> Result<JobServiceClient<Channel>, tonic::transport::Error> {
        self.named_client(address, if authenticated { Some("client") } else { None })
            .await
    }

    pub(super) async fn named_client(
        &self,
        address: std::net::SocketAddr,
        identity: Option<&str>,
    ) -> Result<JobServiceClient<Channel>, tonic::transport::Error> {
        let mut tls = ClientTlsConfig::new()
            .domain_name("localhost")
            .ca_certificate(Certificate::from_pem(self.read("ca.pem")));
        if let Some(identity) = identity {
            tls = tls.identity(Identity::from_pem(
                self.read(&format!("{identity}.pem")),
                self.read(&format!("{identity}.key")),
            ));
        }
        let channel = Endpoint::from_shared(format!("https://localhost:{}", address.port()))
            .unwrap()
            .tls_config(tls)?
            .connect_timeout(std::time::Duration::from_secs(3))
            .timeout(std::time::Duration::from_secs(30))
            .connect()
            .await?;
        Ok(JobServiceClient::new(channel))
    }
}

fn openssl(directory: &Path, arguments: &[&str]) {
    assert!(
        Command::new("openssl")
            .args(arguments)
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success(),
        "temporary TLS fixture generation failed"
    );
}
