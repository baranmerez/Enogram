#![windows_subsystem = "windows"]

use serde_json::Value;

#[tauri::command]
async fn inspect_repository(path: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = enogram_core::inspect_repository(&path).map_err(|error| error.to_string())?;
        serde_json::to_value(snapshot).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn search_repository_history(repo_path: String, query: String, cursor: Option<enogram_core::HistorySearchCursor>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let page = enogram_core::search_repository_history(&repo_path, &query, cursor.as_ref()).map_err(|error| error.to_string())?;
        serde_json::to_value(page).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn read_diff(repo_path: String, file_path: String, staged: bool, commit_oid: Option<String>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let diff = enogram_core::read_diff(&repo_path, &file_path, staged, commit_oid.as_deref()).map_err(|error| error.to_string())?;
        serde_json::to_value(diff).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn prepare_partial_diff(repo_path: String, file_path: String, staged: bool, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let diff = enogram_core::prepare_partial_diff(&repo_path, &file_path, staged, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(diff).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn apply_partial_selection(repo_path: String, file_path: String, staged: bool, expected_token: String, selection_token: String, line_ids: Vec<usize>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::apply_partial_selection(&repo_path, &file_path, staged, &expected_token, &selection_token, &line_ids).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_commit_files(repo_path: String, commit_oid: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = enogram_core::list_commit_files(&repo_path, &commit_oid).map_err(|error| error.to_string())?;
        serde_json::to_value(files).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn stage_paths(repo_path: String, paths: Vec<String>, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::stage_paths(&repo_path, &paths, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn unstage_paths(repo_path: String, paths: Vec<String>, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::unstage_paths(&repo_path, &paths, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn commit_index(repo_path: String, message: String, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::commit_index(&repo_path, &message, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn create_branch(repo_path: String, name: String, checkout: bool, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::create_branch(&repo_path, &name, checkout, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn switch_branch(repo_path: String, name: String, expected_head: String, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::switch_branch(&repo_path, &name, &expected_head, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn add_remote(repo_path: String, name: String, fetch_url: String, push_url: Option<String>, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::add_remote(&repo_path, &name, &fetch_url, push_url.as_deref(), &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn update_remote(repo_path: String, remote_name: String, fetch_url: Option<String>, push_url: Option<String>, update_push_url: bool, expected_remote_token: String, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::update_remote(&repo_path, &remote_name, fetch_url.as_deref(), push_url.as_deref(), update_push_url, &expected_remote_token, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn fetch_remote(repo_path: String, remote_name: String, expected_remote_token: String, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::fetch_remote(&repo_path, &remote_name, &expected_remote_token, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn pull_remote(repo_path: String, remote_name: String, branch_name: String, expected_remote_token: String, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::pull_remote(&repo_path, &remote_name, &branch_name, &expected_remote_token, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
async fn push_remote(repo_path: String, remote_name: String, branch_name: String, expected_remote_token: String, expected_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = enogram_core::push_remote(&repo_path, &remote_name, &branch_name, &expected_remote_token, &expected_token).map_err(|error| error.to_string())?;
        serde_json::to_value(outcome).map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            inspect_repository, search_repository_history, read_diff, prepare_partial_diff, apply_partial_selection, get_commit_files, stage_paths, unstage_paths, commit_index,
            create_branch, switch_branch, add_remote, update_remote, fetch_remote, pull_remote, push_remote
        ])
        .run(tauri::generate_context!())
        .expect("Enogram could not start");
}
