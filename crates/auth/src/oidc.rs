use std::time::Duration;

use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreIdTokenVerifier, CoreTokenResponse},
    reqwest, AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl,
    Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
    TokenResponse,
};
use subtle::ConstantTimeEq;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid authentication configuration")]
    Configuration,
    #[error("identity provider unavailable or invalid discovery metadata")]
    Provider,
    #[error("invalid or expired login attempt")]
    InvalidLogin,
    #[error("authorization code exchange failed")]
    Exchange,
    #[error("invalid identity token")]
    InvalidToken,
    #[error("permission denied")]
    Forbidden,
}

/// Secrets are deliberately excluded from Debug output.
pub struct AuthConfig {
    issuer: IssuerUrl,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect: RedirectUrl,
    allow_loopback_http: bool,
    keycloak: bool,
}

impl AuthConfig {
    /// Enforce verified email and online upstream session validation.
    pub fn keycloak(mut self) -> Self {
        self.keycloak = true;
        self
    }

    /// HTTPS is mandatory unless HTTP on literal loopback is explicitly enabled.
    pub fn new(
        issuer: String,
        client_id: String,
        client_secret: String,
        redirect: String,
        allow_loopback_http: bool,
    ) -> Result<Self, AuthError> {
        if client_id.trim().is_empty() || client_secret.trim().is_empty() {
            return Err(AuthError::Configuration);
        }
        let issuer = IssuerUrl::new(issuer).map_err(|_| AuthError::Configuration)?;
        let redirect = RedirectUrl::new(redirect).map_err(|_| AuthError::Configuration)?;
        validate_url(issuer.url(), allow_loopback_http)?;
        validate_url(redirect.url(), allow_loopback_http)?;
        if issuer.url().query().is_some() {
            return Err(AuthError::Configuration);
        }
        Ok(Self {
            issuer,
            client_id: ClientId::new(client_id),
            client_secret: ClientSecret::new(client_secret),
            redirect,
            allow_loopback_http,
            keycloak: false,
        })
    }
}

fn validate_url(url: &openidconnect::url::Url, allow_loopback_http: bool) -> Result<(), AuthError> {
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (allow_loopback_http && loopback && url.scheme() == "http"))
    {
        return Err(AuthError::Configuration);
    }
    Ok(())
}

/// Verified identity. Construction is private to the validated OIDC flow.
#[derive(Debug, Clone)]
pub struct Identity {
    issuer: String,
    subject: String,
    expires_at: i64,
    pub(crate) access_token: Option<openidconnect::AccessToken>,
    pub(crate) email_verified: bool,
}

impl Identity {
    /// Check that this verified identity is still valid for local account lookup.
    pub fn validate_current(&self) -> Result<(), crate::IdentityError> {
        crate::identity::validate_identity(self)
    }

    pub fn email_verified(&self) -> bool {
        self.email_verified
    }
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    pub fn subject(&self) -> &str {
        &self.subject
    }
    /// Unix timestamp; sessions must never outlive this value without reauthentication.
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }

    pub(crate) fn from_stored(
        issuer: String,
        subject: String,
        expires_at: i64,
    ) -> Result<Self, crate::SessionError> {
        let parsed = IssuerUrl::new(issuer.clone()).map_err(|_| crate::SessionError::Invalid)?;
        validate_url(parsed.url(), true).map_err(|_| crate::SessionError::Invalid)?;
        if subject.is_empty() || subject.len() > 4096 || expires_at <= 0 {
            return Err(crate::SessionError::Invalid);
        }
        Ok(Self {
            issuer,
            subject,
            expires_at,
            access_token: None,
            email_verified: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn set_test_expiration(&mut self, expires_at: i64) {
        self.expires_at = expires_at;
    }

    #[cfg(test)]
    pub(crate) fn for_test(issuer: &str, subject: &str) -> Self {
        Self {
            issuer: issuer.into(),
            subject: subject.into(),
            expires_at: i64::MAX,
            access_token: None,
            email_verified: false,
        }
    }
}

/// Store server-side, bound to the initiating browser session. Never expose or log secrets.
/// Consumed by `finish_login`, including on errors, to prevent reuse.
pub struct LoginAttempt {
    state: CsrfToken,
    nonce: Nonce,
    verifier: PkceCodeVerifier,
    pub(crate) expires_at: u64,
}

impl LoginAttempt {
    pub(crate) fn into_record(self) -> crate::records::LoginRecord {
        crate::records::LoginRecord {
            version: 1,
            state: self.state.secret().to_owned(),
            nonce: self.nonce.secret().to_owned(),
            verifier: self.verifier.secret().to_owned(),
            expires_at: self.expires_at,
        }
    }
    pub(crate) fn from_record(record: crate::records::LoginRecord) -> Self {
        Self {
            state: CsrfToken::new(record.state),
            nonce: Nonce::new(record.nonce),
            verifier: PkceCodeVerifier::new(record.verifier),
            expires_at: record.expires_at,
        }
    }
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        let (_, verifier) = PkceCodeChallenge::new_random_sha256();
        Self {
            state: CsrfToken::new_random(),
            nonce: Nonce::new_random(),
            verifier,
            expires_at: crate::session::unix_now() + 300,
        }
    }
    fn validate(&self, state: &str) -> Result<(), AuthError> {
        if self.expires_at <= crate::session::unix_now()
            || !bool::from(self.state.secret().as_bytes().ct_eq(state.as_bytes()))
        {
            return Err(AuthError::InvalidLogin);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ProviderExtras {
    introspection_endpoint: Option<String>,
}
impl openidconnect::AdditionalProviderMetadata for ProviderExtras {}
type ProviderMetadata = openidconnect::ProviderMetadata<
    ProviderExtras,
    openidconnect::core::CoreAuthDisplay,
    openidconnect::core::CoreClientAuthMethod,
    openidconnect::core::CoreClaimName,
    openidconnect::core::CoreClaimType,
    openidconnect::core::CoreGrantType,
    openidconnect::core::CoreJweContentEncryptionAlgorithm,
    openidconnect::core::CoreJweKeyManagementAlgorithm,
    openidconnect::core::CoreJsonWebKey,
    openidconnect::core::CoreResponseMode,
    openidconnect::core::CoreResponseType,
    openidconnect::core::CoreSubjectIdentifierType,
>;

pub struct OidcAuth {
    config: AuthConfig,
    metadata: ProviderMetadata,
    http: reqwest::Client,
}

impl OidcAuth {
    /// Discovery loads signing keys. Recreate the client periodically to pick up
    /// provider key rotation; unknown keys fail closed until metadata is refreshed.
    pub async fn discover(config: AuthConfig) -> Result<Self, AuthError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| AuthError::Configuration)?;
        let metadata = ProviderMetadata::discover_async(config.issuer.clone(), &http)
            .await
            .map_err(|_| AuthError::Provider)?;
        validate_url(
            metadata.authorization_endpoint().url(),
            config.allow_loopback_http,
        )?;
        let token_endpoint = metadata.token_endpoint().ok_or(AuthError::Provider)?;
        validate_url(token_endpoint.url(), config.allow_loopback_http)?;
        validate_url(metadata.jwks_uri().url(), config.allow_loopback_http)?;
        if config.keycloak {
            let endpoint = metadata
                .additional_metadata()
                .introspection_endpoint
                .as_ref()
                .ok_or(AuthError::Provider)?;
            let endpoint =
                openidconnect::url::Url::parse(endpoint).map_err(|_| AuthError::Provider)?;
            validate_url(&endpoint, config.allow_loopback_http)?;
            if endpoint.origin() != config.issuer.url().origin() {
                return Err(AuthError::Configuration);
            }
        }
        Ok(Self {
            config,
            metadata,
            http,
        })
    }

    pub async fn probe(&self) -> Result<(), AuthError> {
        if !self.config.keycloak {
            return Ok(());
        }
        let response = self
            .http
            .get(self.metadata.jwks_uri().as_str())
            .send()
            .await
            .map_err(|_| AuthError::Provider)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(AuthError::Provider)
        }
    }
    /// Tokens remain server-side. Inactive upstream sessions fail closed.
    pub async fn validate_session(&self, identity: &Identity) -> Result<(), AuthError> {
        if identity.issuer() != self.config.issuer.as_str() {
            return Err(AuthError::InvalidToken);
        }
        if !self.config.keycloak {
            return Ok(());
        }
        #[derive(serde::Deserialize)]
        struct Introspection {
            active: bool,
            sub: Option<String>,
            iss: Option<String>,
            client_id: Option<String>,
            exp: Option<i64>,
        }
        let token = identity
            .access_token
            .as_ref()
            .ok_or(AuthError::InvalidToken)?;
        let endpoint = self
            .metadata
            .additional_metadata()
            .introspection_endpoint
            .as_ref()
            .ok_or(AuthError::Provider)?;
        let response = self
            .http
            .post(endpoint)
            .basic_auth(
                self.config.client_id.as_str(),
                Some(self.config.client_secret.secret()),
            )
            .form(&[
                ("token", token.secret().as_str()),
                ("token_type_hint", "access_token"),
            ])
            .send()
            .await
            .map_err(|_| AuthError::Provider)?;
        if !response.status().is_success() {
            return Err(AuthError::Provider);
        }
        let bytes = response.bytes().await.map_err(|_| AuthError::Provider)?;
        if bytes.len() > 16384 {
            return Err(AuthError::Provider);
        }
        let record: Introspection =
            serde_json::from_slice(&bytes).map_err(|_| AuthError::Provider)?;
        if !record.active
            || record.sub.as_deref() != Some(identity.subject())
            || record.iss.as_deref() != Some(identity.issuer())
            || record.client_id.as_deref() != Some(self.config.client_id.as_str())
            || record
                .exp
                .is_none_or(|exp| exp <= crate::session::unix_now() as i64)
        {
            return Err(AuthError::InvalidToken);
        }
        Ok(())
    }
    pub fn begin_signup(&self) -> (openidconnect::url::Url, LoginAttempt) {
        let (mut url, attempt) = self.begin_login();
        url.query_pairs_mut().append_pair("prompt", "create");
        (url, attempt)
    }
    pub fn begin_recovery(&self) -> Result<(openidconnect::url::Url, LoginAttempt), AuthError> {
        if !self.config.keycloak {
            return Err(AuthError::Configuration);
        }
        let (mut url, attempt) = self.begin_login();
        let path = url
            .path()
            .strip_suffix("/auth")
            .ok_or(AuthError::Configuration)?
            .to_owned()
            + "/forgot-credentials";
        url.set_path(&path);
        Ok((url, attempt))
    }
    pub fn begin_login(&self) -> (openidconnect::url::Url, LoginAttempt) {
        let client = CoreClient::from_provider_metadata(
            self.metadata.clone(),
            self.config.client_id.clone(),
            Some(self.config.client_secret.clone()),
        )
        .set_redirect_uri(self.config.redirect.clone());
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("profile".into()))
            .add_scope(Scope::new("email".into()))
            .set_pkce_challenge(challenge)
            .url();
        (
            url,
            LoginAttempt {
                state,
                nonce,
                verifier,
                expires_at: crate::session::unix_now().saturating_add(crate::session::LOGIN_TTL),
            },
        )
    }

    /// Caller must atomically remove the browser-bound attempt from its store
    /// before invoking this method. No raw tokens escape this boundary.
    pub async fn finish_login(
        &self,
        attempt: LoginAttempt,
        state: &str,
        code: String,
    ) -> Result<Identity, AuthError> {
        attempt.validate(state)?;
        if code.is_empty() {
            return Err(AuthError::InvalidLogin);
        }
        let client = CoreClient::from_provider_metadata(
            self.metadata.clone(),
            self.config.client_id.clone(),
            Some(self.config.client_secret.clone()),
        )
        .set_redirect_uri(self.config.redirect.clone());
        let response = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|_| AuthError::Configuration)?
            .set_pkce_verifier(attempt.verifier)
            .request_async(&self.http)
            .await
            .map_err(|error| match error {
                openidconnect::RequestTokenError::Request(_) => AuthError::Provider,
                _ => AuthError::Exchange,
            })?;
        let verifier = client.id_token_verifier();
        let mut identity = verify_response(&response, &verifier, &attempt.nonce)?;
        if self.config.keycloak {
            let claims = response
                .id_token()
                .ok_or(AuthError::InvalidToken)?
                .claims(&verifier, &attempt.nonce)
                .map_err(|_| AuthError::InvalidToken)?;
            if claims.email_verified() != Some(true) {
                return Err(AuthError::Forbidden);
            }
        }
        identity.access_token = Some(response.access_token().clone());
        Ok(identity)
    }
}

fn verify_response(
    response: &CoreTokenResponse,
    verifier: &CoreIdTokenVerifier<'_>,
    nonce: &Nonce,
) -> Result<Identity, AuthError> {
    let verifier = verifier
        .clone()
        .set_allowed_algs([openidconnect::core::CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256]);
    let token = response.id_token().ok_or(AuthError::InvalidToken)?;
    let claims = token
        .claims(&verifier, nonce)
        .map_err(|_| AuthError::InvalidToken)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| AuthError::InvalidToken)?
        .as_secs() as i64;
    if claims.issue_time().timestamp() > now + 60 {
        return Err(AuthError::InvalidToken);
    }
    if claims.subject().as_str().is_empty() {
        return Err(AuthError::InvalidToken);
    }
    if let Some(expected) = claims.access_token_hash() {
        let actual = AccessTokenHash::from_token(
            response.access_token(),
            token.signing_alg().map_err(|_| AuthError::InvalidToken)?,
            token
                .signing_key(&verifier)
                .map_err(|_| AuthError::InvalidToken)?,
        )
        .map_err(|_| AuthError::InvalidToken)?;
        if actual != *expected {
            return Err(AuthError::InvalidToken);
        }
    }
    Ok(Identity {
        issuer: claims.issuer().as_str().into(),
        subject: claims.subject().as_str().into(),
        expires_at: claims.expiration().timestamp(),
        access_token: None,
        email_verified: claims.email_verified() == Some(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_requires_tls_and_explicit_local_development() {
        let config = |issuer: &str, local| {
            AuthConfig::new(
                issuer.into(),
                "rust-app".into(),
                "secret".into(),
                "https://app.example/callback".into(),
                local,
            )
        };
        assert!(config("https://dex.example", false).is_ok());
        assert!(config("http://localhost:5556", false).is_err());
        assert!(config("http://localhost:5556", true).is_ok());
        assert!(config("http://dex.example", true).is_err());
        assert!(config("https://user:secret@dex.example", false).is_err());
        assert!(config("https://dex.example?query=bad", false).is_err());
        assert!(config("https://dex.example#fragment", false).is_err());
    }

    #[test]
    fn state_and_login_deadline_are_enforced() {
        let (_, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut attempt = LoginAttempt {
            state: CsrfToken::new("expected".into()),
            nonce: Nonce::new_random(),
            verifier,
            expires_at: crate::session::unix_now().saturating_add(crate::session::LOGIN_TTL),
        };
        assert!(attempt.validate("expected").is_ok());
        assert!(attempt.validate("wrong").is_err());
        attempt.expires_at = crate::session::unix_now().saturating_sub(1);
        assert!(attempt.validate("expected").is_err());
    }
    fn signed_response(
        issuer: &str,
        audience: &str,
        nonce: &str,
        expires_in: i64,
        substituted_access_token: bool,
    ) -> (CoreTokenResponse, CoreIdTokenVerifier<'static>) {
        use openidconnect::core::{
            CoreIdToken, CoreIdTokenClaims, CoreIdTokenFields, CoreJsonWebKeySet,
            CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey, CoreTokenType,
        };
        use openidconnect::{
            AccessToken, Audience, EmptyAdditionalClaims, EmptyExtraTokenFields, JsonWebKeyId,
            PrivateSigningKey, StandardClaims, SubjectIdentifier,
        };
        let key = CoreRsaPrivateSigningKey::from_pem(
            include_str!("../tests/fixtures/test-only-rsa.pem"),
            Some(JsonWebKeyId::new("test-key".into())),
        )
        .unwrap();
        let access = AccessToken::new("original-access-token".into());
        let claims = CoreIdTokenClaims::new(
            IssuerUrl::new(issuer.into()).unwrap(),
            vec![Audience::new(audience.into())],
            chrono::Utc::now() + chrono::Duration::seconds(expires_in),
            chrono::Utc::now(),
            StandardClaims::new(SubjectIdentifier::new("alice".into())),
            EmptyAdditionalClaims {},
        )
        .set_nonce(Some(Nonce::new(nonce.into())));
        let token = CoreIdToken::new(
            claims,
            &key,
            CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            Some(&access),
            None,
        )
        .unwrap();
        let verifier = CoreIdTokenVerifier::new_public_client(
            ClientId::new("rust-app".into()),
            IssuerUrl::new("https://dex.example".into()).unwrap(),
            CoreJsonWebKeySet::new(vec![key.as_verification_key()]),
        );
        let access = if substituted_access_token {
            AccessToken::new("substituted".into())
        } else {
            access
        };
        (
            CoreTokenResponse::new(
                access,
                CoreTokenType::Bearer,
                CoreIdTokenFields::new(Some(token), EmptyExtraTokenFields {}),
            ),
            verifier,
        )
    }

    #[test]
    fn signed_tokens_require_issuer_audience_nonce_expiration_and_access_token_binding() {
        for (issuer, audience, nonce, expiration, substituted, accepted) in [
            (
                "https://dex.example",
                "rust-app",
                "expected",
                300,
                false,
                true,
            ),
            (
                "https://attacker.example",
                "rust-app",
                "expected",
                300,
                false,
                false,
            ),
            (
                "https://dex.example",
                "another-app",
                "expected",
                300,
                false,
                false,
            ),
            (
                "https://dex.example",
                "rust-app",
                "wrong",
                300,
                false,
                false,
            ),
            (
                "https://dex.example",
                "rust-app",
                "expected",
                -300,
                false,
                false,
            ),
            (
                "https://dex.example",
                "rust-app",
                "expected",
                300,
                true,
                false,
            ),
        ] {
            let (response, verifier) =
                signed_response(issuer, audience, nonce, expiration, substituted);
            let result = verify_response(&response, &verifier, &Nonce::new("expected".into()));
            assert_eq!(
                result.is_ok(),
                accepted,
                "{issuer} {audience} {nonce} {expiration} {substituted}"
            );
        }
    }

    #[test]
    fn unknown_signing_key_is_rejected() {
        use openidconnect::core::CoreJsonWebKeySet;
        let (response, _) =
            signed_response("https://dex.example", "rust-app", "expected", 300, false);
        let verifier = CoreIdTokenVerifier::new_public_client(
            ClientId::new("rust-app".into()),
            IssuerUrl::new("https://dex.example".into()).unwrap(),
            CoreJsonWebKeySet::new(vec![]),
        );
        assert!(verify_response(&response, &verifier, &Nonce::new("expected".into())).is_err());
    }
    #[test]
    fn tampered_signature_is_rejected() {
        use openidconnect::core::{CoreIdToken, CoreIdTokenFields, CoreTokenType};
        use openidconnect::EmptyExtraTokenFields;
        let (response, verifier) =
            signed_response("https://dex.example", "rust-app", "expected", 300, false);
        let encoded = response.id_token().unwrap().to_string();
        let mut segments: Vec<String> = encoded.split('.').map(str::to_owned).collect();
        let replacement = if segments[2].starts_with('A') {
            "B"
        } else {
            "A"
        };
        segments[2].replace_range(..1, replacement);
        let tampered: CoreIdToken = segments.join(".").parse().unwrap();
        let response = CoreTokenResponse::new(
            response.access_token().clone(),
            CoreTokenType::Bearer,
            CoreIdTokenFields::new(Some(tampered), EmptyExtraTokenFields {}),
        );
        assert!(verify_response(&response, &verifier, &Nonce::new("expected".into())).is_err());
    }

    #[test]
    fn authorization_rejects_expired_identity() {
        let identity = Identity {
            issuer: "https://dex.example".into(),
            subject: "alice".into(),
            expires_at: 1,
            access_token: None,
            email_verified: false,
        };
        let mut policy = crate::Policy::default();
        policy.grant(
            identity.issuer(),
            identity.subject(),
            crate::Permission::ReadBudget,
        );
        assert!(matches!(
            policy.require(&identity, crate::Permission::ReadBudget),
            Err(AuthError::InvalidToken)
        ));
    }

    #[test]
    fn login_requests_use_independent_state_nonce_and_s256_pkce() {
        use openidconnect::core::{
            CoreJwsSigningAlgorithm, CoreResponseType, CoreSubjectIdentifierType,
        };
        use openidconnect::{AuthUrl, JsonWebKeySetUrl, ResponseTypes, TokenUrl};
        let metadata = ProviderMetadata::new(
            IssuerUrl::new("https://dex.example".into()).unwrap(),
            AuthUrl::new("https://dex.example/auth".into()).unwrap(),
            JsonWebKeySetUrl::new("https://dex.example/keys".into()).unwrap(),
            vec![ResponseTypes::new(vec![CoreResponseType::Code])],
            vec![CoreSubjectIdentifierType::Public],
            vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
            ProviderExtras::default(),
        )
        .set_token_endpoint(Some(
            TokenUrl::new("https://dex.example/token".into()).unwrap(),
        ));
        let auth = OidcAuth {
            config: AuthConfig::new(
                "https://dex.example".into(),
                "rust-app".into(),
                "secret".into(),
                "https://app.example/callback".into(),
                false,
            )
            .unwrap(),
            metadata,
            http: reqwest::Client::new(),
        };
        let (url, attempt) = auth.begin_login();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["code_challenge_method"], "S256");
        assert!(query["scope"].split_whitespace().any(|s| s == "openid"));
        assert_eq!(query["nonce"], *attempt.nonce.secret());
        assert!(attempt.validate(&query["state"]).is_ok());
        assert_ne!(attempt.state.secret(), auth.begin_login().1.state.secret());
        assert_eq!(
            query["code_challenge"],
            *PkceCodeChallenge::from_code_verifier_sha256(&attempt.verifier).as_str()
        );
    }
}
