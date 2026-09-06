fn validate_manifest(manifest: &ConnectorManifest) -> Result<()> {
    let schema_version = semver::Version::parse(&manifest.schema_version)
        .context("connector schemaVersion must be strict SemVer")?;
    if schema_version != semver::Version::new(3, 0, 0)
        || schema_version.to_string() != manifest.schema_version
    {
        bail!(
            "connector schemaVersion `{}` is not supported; expected 3.0.0",
            manifest.schema_version
        );
    }
    if manifest.app_id.trim().is_empty() {
        bail!("connector appId cannot be empty");
    }
    if manifest.name.trim().is_empty() {
        bail!("connector name cannot be empty");
    }
    let version = semver::Version::parse(&manifest.version)
        .context("connector version must be strict SemVer")?;
    if version.to_string() != manifest.version {
        bail!("connector version must use canonical SemVer");
    }
    if manifest.transport.is_none() {
        bail!("connector schemaVersion 3.0.0 requires transport");
    }
    if manifest.methods.is_empty() && manifest.events.is_empty() {
        bail!("connector must declare at least one method or event");
    }
    let runtime = manifest
        .runtime
        .as_ref()
        .context("connector schemaVersion 3.0.0 requires runtime")?;
    if runtime.runtime_type != "process" {
        bail!("connector runtime.type must be process");
    }
    if runtime
        .command
        .as_deref()
        .is_none_or(|command| command.trim().is_empty())
    {
        bail!("connector runtime.command cannot be empty");
    }
    validate_host_managed_runtime(manifest, runtime)?;
    validate_app_id(&manifest.app_id)?;
    if let Some(icon) = manifest.icon.as_ref() {
        validate_connector_icon(icon)?;
    }
    if let Some(management) = manifest.management.as_ref() {
        validate_management(management)?;
    }
    if let Some(setup) = manifest.setup.as_ref() {
        let management = manifest
            .management
            .as_ref()
            .context("connector setup requires management")?;
        validate_setup(setup, management)?;
    }
    validate_host_requirements(manifest.host_requirements.as_ref())?;
    validate_managed_tool_dependencies(manifest)?;
    if let Some(ui) = manifest.ui.as_ref() {
        validate_connector_ui(ui)?;
    }
    let start_policy = manifest
        .runtime
        .as_ref()
        .map(|runtime| runtime.start_policy.trim())
        .unwrap_or("automatic");
    if !matches!(start_policy, "automatic" | "manual") {
        bail!("connector runtime.startPolicy must be automatic or manual");
    }
    if let Some(database) = manifest.database.as_ref() {
        validate_database_contract(database)?;
    }
    if let Some(review) = manifest.upgrade_review.as_ref() {
        validate_upgrade_review_contract(manifest, review)?;
    }
    validate_manifest_permissions(&manifest.permissions)?;
    Ok(())
}

fn validate_host_managed_runtime(
    manifest: &ConnectorManifest,
    runtime: &ConnectorRuntime,
) -> Result<()> {
    if runtime.process_ownership != ConnectorProcessOwnership::Host {
        return Ok(());
    }
    if runtime.args.is_empty() {
        bail!("host-managed connector runtime.args must declare a foreground process command");
    }
    if runtime.stop_args.is_empty() {
        bail!("host-managed connector runtime.stopArgs must declare an idempotent graceful shutdown command");
    }
    let requirements = manifest.host_requirements.as_ref().context(
        "host-managed connector requires hostRequirements with an explicit minimumVersion and capability",
    )?;
    let minimum_version = requirements
        .minimum_version
        .as_deref()
        .context("host-managed connector hostRequirements.minimumVersion is required")?;
    let minimum_version = semver::Version::parse(minimum_version).with_context(|| {
        "host-managed connector hostRequirements.minimumVersion must be valid SemVer"
    })?;
    if minimum_version < semver::Version::parse(HOST_MANAGED_PROCESS_MINIMUM_VERSION)? {
        bail!("host-managed connector requires hostRequirements.minimumVersion >= {HOST_MANAGED_PROCESS_MINIMUM_VERSION}");
    }
    if !requirements
        .capabilities
        .iter()
        .any(|capability| capability == HOST_MANAGED_PROCESS_CAPABILITY)
    {
        bail!("host-managed connector requires host capability `{HOST_MANAGED_PROCESS_CAPABILITY}`");
    }
    Ok(())
}

fn validate_host_requirements(requirements: Option<&ConnectorHostRequirements>) -> Result<()> {
    let Some(requirements) = requirements else {
        return Ok(());
    };
    if requirements.minimum_version.is_none() && requirements.capabilities.is_empty() {
        bail!("connector hostRequirements must declare minimumVersion or capabilities");
    }
    if let Some(version) = requirements.minimum_version.as_deref() {
        let minimum_version = semver::Version::parse(version)
            .with_context(|| "connector hostRequirements.minimumVersion must be semver")?;
        let current_version = semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .context("bridge-agent package version must be semver")?;
        if current_version < minimum_version {
            bail!(
                "Connector 要求百积木客户端 {version} 或更高版本，当前版本为 {}，请先升级客户端",
                env!("CARGO_PKG_VERSION")
            );
        }
    }
    if requirements
        .capabilities
        .iter()
        .any(|capability| capability.trim().is_empty() || capability.trim() != capability)
    {
        bail!("connector hostRequirements.capabilities must contain non-empty names");
    }
    let supported = CONNECTOR_HOST_CAPABILITIES.iter().copied().collect::<BTreeSet<_>>();
    let missing = requirements
        .capabilities
        .iter()
        .filter(|capability| !supported.contains(capability.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!("当前百积木客户端缺少 Connector 所需能力：{}，请先升级客户端", missing.join("、"));
    }
    Ok(())
}

fn validate_manifest_permissions(permissions: &[ConnectorPermission]) -> Result<()> {
    let mut permission_ids = BTreeSet::new();
    for permission in permissions {
        if permission.id.trim().is_empty() || permission.title.trim().is_empty() {
            bail!("connector permission id and title cannot be empty");
        }
        if permission.id.trim() != permission.id
            || !permission.id.chars().all(|ch| {
                ch.is_ascii_alphabetic() || ch.is_ascii_digit() || matches!(ch, '.' | '-' | '_')
            })
        {
            bail!("connector permission id must use ASCII letters, digits, dot, dash or underscore");
        }
        if !permission_ids.insert(permission.id.as_str()) {
            bail!("connector permission id `{}` is duplicated", permission.id);
        }
    }
    Ok(())
}

pub fn connector_icon_data_url(icon: &ConnectorIcon) -> Result<String> {
    validate_connector_icon(icon)?;
    Ok(format!(
        "data:{CONNECTOR_ICON_MEDIA_TYPE};base64,{}",
        icon.data
    ))
}

fn validate_connector_icon(icon: &ConnectorIcon) -> Result<()> {
    if icon.media_type != CONNECTOR_ICON_MEDIA_TYPE {
        bail!("connector icon.mediaType must be {CONNECTOR_ICON_MEDIA_TYPE}");
    }
    if icon.data.trim() != icon.data || icon.data.is_empty() {
        bail!("connector icon.data must be non-empty canonical base64");
    }
    let bytes = BASE64_STANDARD
        .decode(&icon.data)
        .context("connector icon.data must be valid base64")?;
    if BASE64_STANDARD.encode(&bytes) != icon.data {
        bail!("connector icon.data must use canonical base64 encoding");
    }
    if bytes.is_empty() || bytes.len() > CONNECTOR_ICON_MAX_BYTES {
        bail!("connector icon PNG must contain 1 to {CONNECTOR_ICON_MAX_BYTES} bytes");
    }
    let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
        .context("connector icon.data must contain a valid PNG image")?;
    if image.width() != CONNECTOR_ICON_EDGE_PX || image.height() != CONNECTOR_ICON_EDGE_PX {
        bail!(
            "connector icon PNG must be {CONNECTOR_ICON_EDGE_PX}x{CONNECTOR_ICON_EDGE_PX} pixels"
        );
    }
    Ok(())
}

fn validate_managed_tool_dependencies(manifest: &ConnectorManifest) -> Result<()> {
    if manifest.managed_tool_dependencies.is_empty() {
        return Ok(());
    }
    let requirements = manifest.host_requirements.as_ref().context(
        "connector managedToolDependencies requires hostRequirements with the managed-tool dependency capability",
    )?;
    if !requirements
        .capabilities
        .iter()
        .any(|capability| capability == MANAGED_TOOL_DEPENDENCIES_CAPABILITY)
    {
        bail!(
            "connector managedToolDependencies requires host capability `{MANAGED_TOOL_DEPENDENCIES_CAPABILITY}`"
        );
    }

    let runtime_env = manifest
        .runtime
        .as_ref()
        .map(|runtime| &runtime.env)
        .context("connector managedToolDependencies requires runtime")?;
    let mut ids = BTreeSet::new();
    let mut environment_variables = BTreeSet::new();
    for dependency in &manifest.managed_tool_dependencies {
        if dependency.id.trim().is_empty() || dependency.id.trim() != dependency.id {
            bail!("connector managedToolDependencies[].id must be a non-empty canonical id");
        }
        validate_app_id(&dependency.id)
            .context("connector managedToolDependencies[].id is invalid")?;
        if !ids.insert(dependency.id.clone()) {
            bail!(
                "connector managedToolDependencies contains duplicate tool id `{}`",
                dependency.id
            );
        }
        semver::Version::parse(dependency.minimum_version.trim()).with_context(|| {
            format!(
                "connector managed tool `{}` minimumVersion must be valid SemVer",
                dependency.id
            )
        })?;
        if dependency.minimum_version.trim() != dependency.minimum_version {
            bail!(
                "connector managed tool `{}` minimumVersion must not contain surrounding whitespace",
                dependency.id
            );
        }
        if dependency.required_for.is_empty() {
            bail!(
                "connector managed tool `{}` requiredFor must contain install or start",
                dependency.id
            );
        }
        let phases = dependency
            .required_for
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if phases.len() != dependency.required_for.len() {
            bail!(
                "connector managed tool `{}` requiredFor contains duplicate phases",
                dependency.id
            );
        }
        if !valid_environment_variable_name(&dependency.executable_path_env) {
            bail!(
                "connector managed tool `{}` executablePathEnv is invalid",
                dependency.id
            );
        }
        if !environment_variables.insert(dependency.executable_path_env.clone()) {
            bail!(
                "connector managedToolDependencies contains duplicate executablePathEnv `{}`",
                dependency.executable_path_env
            );
        }
        if runtime_env.contains_key(&dependency.executable_path_env) {
            bail!(
                "connector runtime.env cannot override host-managed dependency variable `{}`",
                dependency.executable_path_env
            );
        }
    }
    Ok(())
}

fn valid_environment_variable_name(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn validate_database_contract(database: &ConnectorDatabaseContract) -> Result<()> {
    if database.engine.trim().is_empty() || database.schema_version.trim().is_empty() {
        bail!("connector database.engine and database.schemaVersion cannot be empty");
    }
    let mut migration_ids = BTreeSet::new();
    for migration in &database.migrations {
        if migration.id.trim().is_empty()
            || migration.from_version.trim().is_empty()
            || migration.to_version.trim().is_empty()
            || migration.description.trim().is_empty()
        {
            bail!(
                "connector database migration id, fromVersion, toVersion and description cannot be empty"
            );
        }
        if migration.from_version == migration.to_version {
            bail!(
                "connector database migration `{}` must change schema version",
                migration.id
            );
        }
        if !migration_ids.insert(migration.id.as_str()) {
            bail!(
                "connector database migration id `{}` is duplicated",
                migration.id
            );
        }
        if !matches!(
            migration.rollback.as_str(),
            "automatic" | "manual" | "unsupported" | "not_declared"
        ) {
            bail!(
                "connector database migration `{}` rollback must be automatic, manual, unsupported or not_declared",
                migration.id
            );
        }
        if !matches!(
            migration.downtime.as_str(),
            "none" | "brief" | "required" | "not_declared"
        ) {
            bail!(
                "connector database migration `{}` downtime must be none, brief, required or not_declared",
                migration.id
            );
        }
        if migration.changes.is_empty() {
            bail!(
                "connector database migration `{}` must declare at least one change",
                migration.id
            );
        }
        for change in &migration.changes {
            if change.operation.trim().is_empty()
                || change.target.trim().is_empty()
                || change.description.trim().is_empty()
            {
                bail!(
                    "connector database migration `{}` changes require operation, target and description",
                    migration.id
                );
            }
        }
    }
    Ok(())
}

fn validate_upgrade_review_contract(
    manifest: &ConnectorManifest,
    review: &ConnectorUpgradeReview,
) -> Result<()> {
    for (name, value) in [
        ("configuration", review.configuration.as_str()),
        ("interfaces", review.interfaces.as_str()),
        ("database", review.database.as_str()),
    ] {
        if !matches!(value, "declared" | "not_applicable") {
            bail!("connector upgradeReview.{name} must be declared or not_applicable");
        }
    }
    if (review.configuration == "declared") != manifest.config_schema.is_some() {
        bail!(
            "connector upgradeReview.configuration must be declared exactly when configSchema is present"
        );
    }
    if review.interfaces != "declared" {
        bail!("connector upgradeReview.interfaces must be declared for schemaVersion 3.0.0");
    }
    if (review.database == "declared") != manifest.database.is_some() {
        bail!("connector upgradeReview.database must be declared exactly when database is present");
    }
    Ok(())
}

fn validate_connector_ui(ui: &ConnectorUi) -> Result<()> {
    if ui.ui_type != "embedded" {
        bail!("connector ui.type must be embedded");
    }
    let entry = validated_connector_ui_relative_path(&ui.entry, "entry")?;
    if entry
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        bail!("connector ui.entry must be inside a dedicated UI directory");
    }
    if entry.extension().and_then(|value| value.to_str()) != Some("html") {
        bail!("connector ui.entry must point to an .html file");
    }
    if ui
        .title
        .as_deref()
        .is_some_and(|title| title.trim().is_empty() || title.chars().count() > 64)
    {
        bail!("connector ui.title must contain 1 to 64 characters");
    }
    Ok(())
}

fn validated_connector_ui_relative_path(value: &str, field: &str) -> Result<PathBuf> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.contains('\\') {
        bail!("connector UI {field} must be a non-empty forward-slash relative path");
    }
    let path = Path::new(normalized);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("connector UI {field} must stay inside the UI directory");
    }
    Ok(path.to_path_buf())
}

fn validate_app_id(app_id: &str) -> Result<()> {
    let app_id = app_id.trim();
    if app_id.is_empty()
        || app_id == "."
        || app_id == ".."
        || sanitize_path_component(app_id) != app_id
    {
        bail!("connector id contains unsupported characters")
    }
    Ok(())
}

fn validate_management(management: &ConnectorManagement) -> Result<()> {
    if !management.management_type.eq_ignore_ascii_case("http") {
        bail!("connector management.type must be http");
    }
    if management.auth.auth_type != "connector_token" {
        bail!("connector management.auth.type must be connector_token");
    }
    let url = url::Url::parse(&management.base_url)
        .with_context(|| "connector management.baseUrl must be a valid URL")?;
    if url.scheme() != "http" || !url.has_host() {
        bail!("connector management.baseUrl must use local HTTP");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("connector management.baseUrl must be an origin-only URL");
    }
    let host = url.host_str().unwrap_or_default();
    if host != "localhost"
        && host != "127.0.0.1"
        && host != "::1"
        && url.host().is_none_or(|host| match host {
            url::Host::Ipv4(address) => !address.is_loopback(),
            url::Host::Ipv6(address) => !address.is_loopback(),
            url::Host::Domain(_) => true,
        })
    {
        bail!("connector management.baseUrl must be loopback-only");
    }
    if management.operations.is_empty() {
        bail!("connector management.operations cannot be empty");
    }
    for (name, operation) in &management.operations {
        if name.trim().is_empty() || sanitize_path_component(name) != *name {
            bail!("connector management operation name is invalid");
        }
        if !matches!(operation.method.as_str(), "GET" | "POST") {
            bail!("connector management operation `{name}` method must be GET or POST");
        }
        if !operation.path.starts_with("/management/")
            || operation.path.contains('?')
            || operation.path.contains('#')
        {
            bail!("connector management operation `{name}` path is invalid");
        }
    }
    Ok(())
}

fn validate_setup(setup: &ConnectorSetup, management: &ConnectorManagement) -> Result<()> {
    if !(30..=3_600).contains(&setup.timeout_secs) {
        bail!("connector setup.timeoutSecs must be between 30 and 3600");
    }
    let operation = management
        .operations
        .get(&setup.operation)
        .with_context(|| {
            format!(
                "connector setup.operation `{}` is not declared by management.operations",
                setup.operation
            )
        })?;
    if operation.method != "POST" {
        bail!("connector setup.operation must use POST");
    }
    let status_operation = management
        .operations
        .get(&setup.status_operation)
        .with_context(|| {
            format!(
                "connector setup.statusOperation `{}` is not declared by management.operations",
                setup.status_operation
            )
        })?;
    if status_operation.method != "GET" {
        bail!("connector setup.statusOperation must use GET");
    }
    Ok(())
}
