#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRegistration {
    pub name: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub transport: RegistrationTransport,
    #[serde(default, alias = "healthCheck")]
    pub health_check: Option<RegistrationHealthCheck>,
    #[serde(default, alias = "startCommand")]
    pub start_command: Option<ServiceStartCommand>,
    #[serde(default, alias = "stopCommand")]
    pub stop_command: Option<ServiceStartCommand>,
    #[serde(default)]
    pub methods: Vec<RegistrationMethod>,
    #[serde(default, rename = "events")]
    pub local_app_events: Vec<EventConfig>,
    #[serde(default)]
    pub replace: bool,
    #[serde(default)]
    pub managed_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RegistrationTransport {
    Http {
        #[serde(alias = "baseUrl")]
        base_url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RegistrationHealthCheck {
    Http {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default = "default_health_check_http_method", alias = "httpMethod")]
        http_method: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, alias = "timeoutSecs")]
        timeout_secs: Option<u64>,
        #[serde(default, alias = "expectStatus")]
        expect_status: Option<u16>,
        #[serde(default, alias = "bodyContains")]
        body_contains: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationMethod {
    pub name: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_object_schema")]
    pub input_schema: Value,
    #[serde(default, rename = "responseMode", alias = "response_mode")]
    pub response_mode: ResponseMode,
    #[serde(default, alias = "path")]
    pub path: String,
    #[serde(default = "default_http_method", alias = "httpMethod")]
    pub http_method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, alias = "timeoutSecs")]
    pub timeout_secs: Option<u64>,
}

impl ServiceRegistration {
    pub fn into_service_config(self) -> Result<ServiceConfig> {
        self.into_service_config_inner(false)
    }

    pub(crate) fn into_connector_service_config(self) -> Result<ServiceConfig> {
        self.into_service_config_inner(true)
    }

    fn into_service_config_inner(self, allow_event_only_connector: bool) -> Result<ServiceConfig> {
        if self.name.trim().is_empty() {
            bail!("service name cannot be empty");
        }
        if self.methods.is_empty()
            && (!allow_event_only_connector || self.local_app_events.is_empty())
        {
            bail!("service registration must include at least one method");
        }

        let (methods, health_check) = match self.transport {
            RegistrationTransport::Http { base_url, headers } => {
                let base_url = normalize_registration_base_url(&base_url)?;
                let health_check = self
                    .health_check
                    .map(|health_check| health_check.into_service_health_check(&base_url, &headers))
                    .transpose()?;
                let methods = self
                    .methods
                    .into_iter()
                    .map(|method| method.into_http_method_config(&base_url, &headers))
                    .collect::<Result<Vec<_>>>()?;
                (methods, health_check)
            }
        };

        Ok(ServiceConfig {
            name: self.name.trim().to_string(),
            description: self.description.trim().to_string(),
            enabled: self.enabled,
            health_check,
            start_command: self.start_command,
            stop_command: self.stop_command,
            methods,
        })
    }
}

impl RegistrationHealthCheck {
    fn into_service_health_check(
        self,
        base_url: &Url,
        transport_headers: &BTreeMap<String, String>,
    ) -> Result<ServiceHealthCheck> {
        match self {
            RegistrationHealthCheck::Http {
                path,
                url,
                http_method,
                headers,
                timeout_secs,
                expect_status,
                body_contains,
            } => {
                let resolved_url = match url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(url) => normalize_registration_base_url(url)?.to_string(),
                    None => join_registration_url(base_url, path.as_deref().unwrap_or("/health"))?,
                };
                let mut resolved_headers = transport_headers.clone();
                resolved_headers.extend(headers);
                Ok(ServiceHealthCheck::Http {
                    url: resolved_url,
                    http_method: http_method.trim().to_uppercase(),
                    headers: resolved_headers,
                    timeout_secs,
                    expect_status,
                    body_contains,
                })
            }
        }
    }
}

impl RegistrationMethod {
    fn into_http_method_config(
        self,
        base_url: &Url,
        transport_headers: &BTreeMap<String, String>,
    ) -> Result<MethodConfig> {
        if self.name.trim().is_empty() {
            bail!("method name cannot be empty");
        }
        let mut headers = transport_headers.clone();
        headers.extend(self.headers);

        Ok(MethodConfig {
            name: self.name.trim().to_string(),
            description: self.description.trim().to_string(),
            enabled: self.enabled,
            input_schema: self.input_schema,
            response_mode: self.response_mode,
            binding: MethodBinding::Http(HttpBinding {
                url: join_registration_url(base_url, &self.path)?,
                http_method: self.http_method.trim().to_uppercase(),
                headers,
                timeout_secs: self.timeout_secs,
            }),
        })
    }
}

fn normalize_registration_base_url(base_url: &str) -> Result<Url> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        bail!("transport.baseUrl cannot be empty");
    }
    let url =
        Url::parse(base_url).with_context(|| format!("invalid transport.baseUrl `{base_url}`"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        bail!("transport.baseUrl must use http or https");
    }
    Ok(url)
}

fn join_registration_url(base_url: &Url, path: &str) -> Result<String> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(base_url.as_str().trim_end_matches('/').to_string());
    }
    let path = path.trim_start_matches('/');
    Ok(base_url
        .join(path)
        .with_context(|| format!("invalid method path `{path}`"))?
        .to_string())
}
