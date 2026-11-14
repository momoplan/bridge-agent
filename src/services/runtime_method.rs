impl RuntimeMethod {
    async fn invoke(&self, arguments: Value, timeout_secs: Option<u64>) -> Result<ServiceOutcome> {
        match self {
            Self::File(method) => method.invoke(arguments).await.map(success_outcome),
            Self::Shell(method) => method.invoke(arguments, timeout_secs).await,
            Self::Http(method) => method.invoke(arguments, timeout_secs).await,
            Self::Computer(method) => method.invoke(arguments).await,
        }
    }

    async fn invoke_local_app(
        &self,
        arguments: Value,
        timeout_secs: Option<u64>,
        workspace_id: Option<u64>,
    ) -> Result<ServiceOutcome> {
        match self {
            Self::File(method) => method.invoke(arguments).await.map(success_outcome),
            Self::Http(method) => {
                method
                    .invoke_with_workspace(arguments, timeout_secs, workspace_id)
                    .await
            }
            Self::Shell(method) => method.invoke(arguments, timeout_secs).await,
            Self::Computer(method) => method.invoke(arguments).await,
        }
    }
}
