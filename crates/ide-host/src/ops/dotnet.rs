//! .NET solutions: what the Solution Explorer shows.
use crate::Host;
use crate::rpc::Dispatcher;

use ide_core::{IdeResult, IpcPath};
use ide_dotnet::Solution;

/// Solution files at the project root, `.slnx` before `.sln` before `.slnf`.
pub async fn dotnet_solutions(host: &Host, window: &str) -> IdeResult<Vec<IpcPath>> {
    let root = host.project(window)?.root.clone();
    host.blocking(move || Ok(ide_dotnet::find(&root).into_iter().map(IpcPath::from).collect())).await
}

/// The solution at `path`: its folders, projects and solution items.
pub async fn dotnet_solution(host: &Host, _window: &str, path: IpcPath) -> IdeResult<Solution> {
    host.blocking(move || ide_dotnet::read(&path)).await
}

pub(crate) fn register(d: &mut Dispatcher) {
    d.add("dotnet_solutions", |c| async move { dotnet_solutions(&c.host, &c.window).await });
    d.add("dotnet_solution", |c| async move { dotnet_solution(&c.host, &c.window, c.arg("path")?).await });
}
