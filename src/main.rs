mod manager;

use crate::manager::Manager;

fn main() -> xcb::Result<()> {
    env_logger::Builder::new().parse_default_env().init();

    let wm = Manager::connect()?;
    wm.run()
}
