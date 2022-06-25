mod manager;
mod window;

use crate::manager::Manager;

fn main() -> xcb::Result<()> {
    env_logger::Builder::new().parse_default_env().init();

    let mut wm = Manager::connect()?;
    wm.attach_existing_windows()?;
    wm.run()
}
