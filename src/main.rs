use xcb::x;
use log::debug;

fn main() -> xcb::Result<()> {
    env_logger::Builder::new().parse_default_env().init();

    // connect to server
    let (conn, scr_num) = xcb::Connection::connect(None)?;

    // get screen handle
    let screen = conn.get_setup().roots().nth(scr_num as usize).unwrap();

    // ask to be the window manager
    conn.check_request(conn.send_request_checked(&x::ChangeWindowAttributes {
        window: screen.root(),
        value_list: &[
            x::Cw::EventMask(
                x::EventMask::SUBSTRUCTURE_REDIRECT |
                x::EventMask::STRUCTURE_NOTIFY |
                x::EventMask::SUBSTRUCTURE_NOTIFY |
                x::EventMask::PROPERTY_CHANGE
            ),
        ],
    }))?;

    // main loop
    loop {
        match conn.wait_for_event()? {

            // client wants to be displayed
            xcb::Event::X(x::Event::MapRequest(ev)) => {
                debug!("MapRequest: {:?}", ev);

                // be visible!
                conn.check_request(conn.send_request_checked(&x::MapWindow {
                    window: ev.window(),
                }))?;

                // position and size
                conn.check_request(conn.send_request_checked(&x::ConfigureWindow {
                    window: ev.window(),
                    value_list: &[
                        x::ConfigWindow::X(0),
                        x::ConfigWindow::Y(0),
                        x::ConfigWindow::Width(640),
                        x::ConfigWindow::Height(480),
                    ],
                }))?;

                // receive the focus
                conn.check_request(conn.send_request_checked(&x::ChangeWindowAttributes {
                    window: ev.window(),
                    value_list: &[
                        x::Cw::EventMask(
                            x::EventMask::ENTER_WINDOW |
                            x::EventMask::FOCUS_CHANGE
                        ),
                    ],
                }))?;
            },

            e => {
                debug!("UNHANDLED: {:?}", e);
            }
        }
    }
}
