#[derive(Debug, Clone)]
pub struct SshTarget {
    pub user: String,
    pub host: String,
    pub port: u16,
}

pub const PUBLIC_KEY_ONLY_AUTH_OPTIONS: [(&str, &str); 4] = [
    ("PreferredAuthentications", "publickey"),
    ("PasswordAuthentication", "no"),
    ("KbdInteractiveAuthentication", "no"),
    ("HostbasedAuthentication", "no"),
];

impl SshTarget {
    pub fn new(user: String, host: String, port: u16) -> Self {
        Self { user, host, port }
    }

    pub fn destination(&self) -> String {
        format!("{}@{}", self.user, self.host)
    }

    pub fn ssh_config(&self) -> String {
        let auth_options = PUBLIC_KEY_ONLY_AUTH_OPTIONS
            .iter()
            .map(|(name, value)| format!("    {name} {value}\n"))
            .collect::<String>();

        format!(
            r#"Host hop
    HostName {host}
    Port {port}
    User {user}
{auth_options}

Host *.hop
    ProxyJump hop
    HostName %h
"#,
            host = self.host,
            port = self.port,
            user = self.user,
            auth_options = auth_options
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_uses_public_key_only_auth_for_hop_host() {
        let target = SshTarget::new("hop".to_string(), "127.0.0.1".to_string(), 2222);
        let config = target.ssh_config();

        assert!(config.contains("PreferredAuthentications publickey"));
        assert!(config.contains("PasswordAuthentication no"));
        assert!(config.contains("KbdInteractiveAuthentication no"));
        assert!(config.contains("HostbasedAuthentication no"));
    }
}
