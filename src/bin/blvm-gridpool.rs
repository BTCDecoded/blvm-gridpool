//! GridPool module. Spawned by blvm-node. Does not hook getblocktemplate.

use std::sync::Arc;

use anyhow::Result;
use blvm_gridpool::module::GridPoolModule;
use blvm_sdk::module::{ModuleBootstrap, ModuleDb};

const MODULE_NAME: &str = "blvm-gridpool";

#[tokio::main]
async fn main() -> Result<()> {
    let bootstrap = ModuleBootstrap::init_module(MODULE_NAME);
    let db = ModuleDb::open_or_temp(&bootstrap.data_dir, MODULE_NAME)?;
    let setup = |node_api: Arc<dyn blvm_node::module::traits::NodeAPI>,
                 _db: Arc<dyn blvm_node::storage::database::Database>,
                 data_dir: &std::path::Path| {
        let data_dir = data_dir.to_path_buf();
        async move {
            let module = GridPoolModule::new(data_dir, Arc::clone(&node_api)).map_err(|error| {
                blvm_node::module::traits::ModuleError::InitializationError(error.to_string())
            })?;
            node_api
                .register_module_api(Arc::new(module.api()))
                .await
                .map_err(|error| {
                    blvm_node::module::traits::ModuleError::InitializationError(error.to_string())
                })?;
            Ok((module.clone(), module))
        }
    };

    blvm_sdk::run_module! {
        bootstrap: &bootstrap,
        module_name: MODULE_NAME,
        module_type: GridPoolModule,
        cli_type: GridPoolModule,
        db: db.as_db(),
        setup: setup,
        event_types: GridPoolModule::event_types(),
    }?;
    Ok(())
}
