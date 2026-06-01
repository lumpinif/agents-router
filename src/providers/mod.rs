#[cfg(test)]
pub(crate) mod contract_test;
pub mod discord;
pub mod email_smtp;
pub mod feishu_lark;
pub(crate) mod feishu_lark_continuation;
pub(crate) mod feishu_lark_control;
pub mod feishu_lark_long_connection;
pub(crate) mod feishu_lark_new_session;
mod formatting;
mod http;
pub mod microsoft_teams;
mod notification_view;
pub mod ntfy;
pub mod pushover;
pub mod slack;
pub mod telegram;
pub mod webhook;
pub mod wechat;
pub mod whatsapp;

use crate::config::{ProviderType, ValidatedConfig};
use crate::router::Provider;

pub use discord::DiscordProvider;
pub use email_smtp::EmailSmtpProvider;
pub use feishu_lark::FeishuLarkProvider;
pub use microsoft_teams::MicrosoftTeamsProvider;
pub use ntfy::NtfyProvider;
pub use pushover::PushoverProvider;
pub use slack::SlackProvider;
pub use telegram::TelegramProvider;
pub use webhook::WebhookProvider;
pub use wechat::WechatProvider;
pub use whatsapp::WhatsappProvider;

pub fn build_providers(config: &ValidatedConfig) -> anyhow::Result<Vec<Box<dyn Provider>>> {
    config
        .providers
        .iter()
        .map(|provider| match provider.provider_type() {
            ProviderType::Ntfy => {
                Ok(Box::new(NtfyProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::Webhook => {
                Ok(Box::new(WebhookProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::FeishuLark => {
                Ok(Box::new(FeishuLarkProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::Pushover => {
                Ok(Box::new(PushoverProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::Slack => {
                Ok(Box::new(SlackProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::Discord => {
                Ok(Box::new(DiscordProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::Telegram => {
                Ok(Box::new(TelegramProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::Whatsapp => {
                Ok(Box::new(WhatsappProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::Wechat => {
                Ok(Box::new(WechatProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::MicrosoftTeams => {
                Ok(Box::new(MicrosoftTeamsProvider::from_config(provider)?) as Box<dyn Provider>)
            }
            ProviderType::EmailSmtp => {
                Ok(Box::new(EmailSmtpProvider::from_config(provider)?) as Box<dyn Provider>)
            }
        })
        .collect::<Result<Vec<_>, anyhow::Error>>()
}

#[cfg(test)]
mod tests {
    use crate::config::{
        CliConfig, LogConfig, NotificationConfig, ProviderType, RawConfig, RawProviderConfig,
        RouteConfig, SourceConfig, SourceType,
    };

    use super::build_providers;

    const MISSING_WEBHOOK_URL_ENV: &str = "__AGENTS_ROUTER_MISSING_WEBHOOK_URL_FOR_PROVIDER_BUILD";

    #[test]
    fn build_providers_preserves_runtime_env_error_chain() {
        let _guard = MissingEnvGuard::new(MISSING_WEBHOOK_URL_ENV);
        let mut provider = RawProviderConfig::new("debug", ProviderType::Webhook);
        provider.url_env = Some(MISSING_WEBHOOK_URL_ENV.to_string());
        let config = RawConfig {
            schema_version: 1,
            cli: CliConfig::default(),
            log: LogConfig::default(),
            notification: NotificationConfig::default(),
            sources: vec![SourceConfig {
                id: "agents_router".to_string(),
                source_type: SourceType::AgentsRouter,
            }],
            providers: vec![provider],
            routes: vec![RouteConfig::new(
                vec!["agents_router".to_string()],
                vec!["debug".to_string()],
            )],
        }
        .validate()
        .expect("env-backed provider config should validate before runtime build");

        let error = match build_providers(&config) {
            Ok(_) => panic!("missing env var should fail at runtime"),
            Err(error) => error,
        };
        let chain = error
            .chain()
            .map(|error| error.to_string())
            .collect::<Vec<_>>();

        assert!(
            chain.iter().any(|message| message.contains(
                "webhook provider `debug` env var `__AGENTS_ROUTER_MISSING_WEBHOOK_URL_FOR_PROVIDER_BUILD` from `url_env` is not set"
            )),
            "provider context should be preserved in error chain: {chain:?}"
        );
        assert!(
            chain.len() > 1,
            "source error should remain in the chain instead of being stringified away: {chain:?}"
        );
    }

    struct MissingEnvGuard {
        name: &'static str,
    }

    impl MissingEnvGuard {
        fn new(name: &'static str) -> Self {
            unsafe { std::env::remove_var(name) };
            Self { name }
        }
    }

    impl Drop for MissingEnvGuard {
        fn drop(&mut self) {
            unsafe { std::env::remove_var(self.name) };
        }
    }
}
