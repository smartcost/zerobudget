//! Manual integration test against the development Keycloak from Docker Compose.
use std::io::{self, Write};

use obz_auth::{AuthConfig, OidcAuth, Permission, Policy};
use openidconnect::url::Url;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let redirect = "http://localhost:3000/api/auth/callback";
    let auth = OidcAuth::discover(
        AuthConfig::new(
            "http://localhost:8080/realms/zerobudget".into(),
            "zerobudget".into(),
            "local-keycloak-client-secret".into(),
            redirect.into(),
            true,
        )?
        .keycloak(),
    )
    .await?;
    let (url, attempt) = auth.begin_login();
    println!("Abra esta URL no navegador:\n{url}\n");
    println!("Login local: admin@zerobudget.local / Local-Password-123!");
    println!("Após autorizar, o navegador pode mostrar erro de conexão: este exemplo não inicia um servidor na porta 3000.");
    println!("Copie a URL completa da barra de endereço e cole abaixo. Não compartilhe essa URL.");
    print!("URL de retorno: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let callback = Url::parse(input.trim())?;
    let mut expected = Url::parse(redirect)?;
    expected.set_query(callback.query());
    if callback != expected {
        return Err("O retorno não corresponde ao callback configurado".into());
    }
    let parameters: Vec<_> = callback.query_pairs().collect();
    if parameters.iter().any(|(key, _)| key == "error") {
        return Err("O Keycloak recusou ou cancelou a autorização".into());
    }
    let parameter = |name: &str| -> Result<String, Box<dyn std::error::Error>> {
        let values: Vec<_> = parameters.iter().filter(|(key, _)| key == name).collect();
        match values.as_slice() {
            [(_, value)] if !value.is_empty() => Ok(value.to_string()),
            _ => Err("Callback com parâmetro obrigatório ausente, vazio ou duplicado".into()),
        }
    };
    let state = parameter("state")?;
    let code = parameter("code")?;
    let identity = auth.finish_login(attempt, &state, code).await?;
    println!("Autenticação validada.");
    println!(
        "Issuer: {}\nSubject: {}\nExpira em (Unix): {}",
        identity.issuer(),
        identity.subject(),
        identity.expires_at()
    );

    let mut policy = Policy::default();
    assert!(policy.require(&identity, Permission::ReadBudget).is_err());
    // Explicit grant only for this local demonstration; production grants come
    // from trusted application configuration, never from the callback.
    policy.grant(
        identity.issuer(),
        identity.subject(),
        Permission::ReadBudget,
    );
    policy.require_owner(
        &identity,
        Permission::ReadBudget,
        identity.issuer(),
        identity.subject(),
    )?;
    assert!(policy
        .require_owner(
            &identity,
            Permission::ReadBudget,
            identity.issuer(),
            "another-user"
        )
        .is_err());
    println!("Autorização validada: negação por padrão, concessão explícita e isolamento de proprietário.");
    Ok(())
}
