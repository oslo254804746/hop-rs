#[derive(Debug, Clone)]
pub struct SshTarget {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl SshTarget {
    pub fn new(user: String, host: String, port: u16) -> Self {
        Self { user, host, port }
    }

    pub fn destination(&self) -> String {
        format!("{}@{}", self.user, self.host)
    }

    pub fn ssh_config(&self) -> String {
        format!(
            r#"Host hop
    HostName {host}
    Port {port}
    User {user}

Host *.hop
    ProxyJump hop
    HostName %h
"#,
            host = self.host,
            port = self.port,
            user = self.user
        )
    }
}
