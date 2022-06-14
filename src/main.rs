use xcb::{x, Xid};
use log::debug;

fn main() -> xcb::Result<()> {
    env_logger::Builder::new().parse_default_env().init();

    // connect to server
    let (conn, scr_num) = xcb::Connection::connect(None)?;

    // get screen handle
    let screen = conn.get_setup().roots().nth(scr_num as usize).unwrap();

    // ask to be the window manager
    conn.send_request_checked(&x::ChangeWindowAttributes {
        window: screen.root(),
        value_list: &[
            x::Cw::EventMask(
                x::EventMask::SUBSTRUCTURE_REDIRECT |
                x::EventMask::STRUCTURE_NOTIFY |
                x::EventMask::SUBSTRUCTURE_NOTIFY |
                x::EventMask::PROPERTY_CHANGE
            ),
        ],
    });

    // release all grabs
    conn.send_request_checked(&x::UngrabKey {
        key: x::GRAB_ANY,
        grab_window: screen.root(),
        modifiers: x::ModMask::ANY,
    });

    // grab Mod4+Left
    conn.send_request_checked(&x::GrabButton {
        owner_events: false,
        grab_window: screen.root(),
        event_mask: x::EventMask::BUTTON_PRESS | x::EventMask::BUTTON_RELEASE,
        pointer_mode: x::GrabMode::Async,
        keyboard_mode: x::GrabMode::Async,
        confine_to: screen.root(),
        cursor: x::CURSOR_NONE,
        button: x::ButtonIndex::N1,
        modifiers: x::ModMask::N4,
    });

    // grab Mod4+Right
    conn.send_request_checked(&x::GrabButton {
        owner_events: false,
        grab_window: screen.root(),
        event_mask: x::EventMask::BUTTON_PRESS | x::EventMask::BUTTON_RELEASE,
        pointer_mode: x::GrabMode::Async,
        keyboard_mode: x::GrabMode::Async,
        confine_to: screen.root(),
        cursor: x::CURSOR_NONE,
        button: x::ButtonIndex::N3,
        modifiers: x::ModMask::N4,
    });

    conn.flush()?;

    #[derive(Clone, Copy)]
    enum ButtonState { Left, Right }
    let mut button_down: Option<(ButtonState, x::Window)> = None;

    // main loop
    loop {
        match conn.wait_for_event()? {

            // client wants to be displayed
            xcb::Event::X(x::Event::MapRequest(ev)) => {
                debug!("MapRequest: {:?}", ev);

                // be visible!
                conn.send_request_checked(&x::MapWindow {
                    window: ev.window(),
                });

                // position and size
                conn.send_request_checked(&x::ConfigureWindow {
                    window: ev.window(),
                    value_list: &[
                        x::ConfigWindow::X(0),
                        x::ConfigWindow::Y(0),
                        x::ConfigWindow::Width(640),
                        x::ConfigWindow::Height(480),
                    ],
                });

                // receive the focus
                conn.send_request_checked(&x::ChangeWindowAttributes {
                    window: ev.window(),
                    value_list: &[
                        x::Cw::EventMask(
                            x::EventMask::ENTER_WINDOW |
                            x::EventMask::FOCUS_CHANGE
                        ),
                    ],
                });

                conn.flush()?;
            },

            // Mod4+button inside window area
            xcb::Event::X(x::Event::ButtonPress(ev)) => {
                debug!("ButtonPress: {:?}", ev);

                // ignore if we're not over a window
                if ev.child().is_none() {
                    continue;
                }

                // bring window to front
                conn.send_request_checked(&x::ConfigureWindow {
                    window: ev.child(),
                    value_list: &[
                        x::ConfigWindow::StackMode(x::StackMode::Above),
                    ],
                });

                // grab the pointer for window move
                conn.send_request(&x::GrabPointer {
                    owner_events: false,
                    grab_window: screen.root(),
                    event_mask: x::EventMask::BUTTON_RELEASE | x::EventMask::BUTTON_MOTION | x::EventMask::POINTER_MOTION_HINT,
                    pointer_mode: x::GrabMode::Async,
                    keyboard_mode: x::GrabMode::Async,
                    confine_to: screen.root(),
                    cursor: x::CURSOR_NONE,
                    time: x::CURRENT_TIME,
                });

                conn.flush()?;

                // record window
                button_down = match ev.detail() {
                    1 => Some((ButtonState::Left, ev.child())),
                    3 => Some((ButtonState::Right, ev.child())),
                    _ => None,
                };
            },

            xcb::Event::X(x::Event::ButtonRelease(ev)) => {
                debug!("ButtonRelease: {:?}", ev);

                // just release the pointer
                conn.send_request_checked(&x::UngrabPointer {
                    time: x::CURRENT_TIME,
                });
                conn.flush()?;

                button_down = None;
            },

            xcb::Event::X(x::Event::MotionNotify(ev)) => {
                debug!("MotionNotify: {:?}", ev);

                if let Some((button, win)) = button_down {
                    let pointer = conn.wait_for_reply(conn.send_request(&x::QueryPointer {
                        window: screen.root(),
                    }))?;
                    let geometry = conn.wait_for_reply(conn.send_request(&x::GetGeometry {
                        drawable: x::Drawable::Window(win),
                    }))?;

                    match button {
                        ButtonState::Left => {

                            // XXX include border width
                            let win_width = geometry.width() as usize;
                            let win_height = geometry.height() as usize;

                            let scr_width = screen.width_in_pixels() as usize;
                            let scr_height = screen.height_in_pixels() as usize;

                            let ptr_x = pointer.root_x() as usize;
                            let ptr_y = pointer.root_y() as usize;

                            let new_x = if ptr_x + win_width > scr_width {
                                scr_width - win_width
                            } else {
                                ptr_x
                            };
                            let new_y = if ptr_y + win_height > scr_height {
                                scr_height - win_height
                            } else {
                                ptr_y
                            };

                            debug!("moving window: {},{}", new_x, new_y);

                            conn.send_request_checked(&x::ConfigureWindow {
                                window: win,
                                value_list: &[
                                    x::ConfigWindow::X(new_x as i32),
                                    x::ConfigWindow::Y(new_y as i32),
                                ],
                            });
                            conn.flush()?;
                        },

                        ButtonState::Right => {

                            let win_x = geometry.x() as usize;
                            let win_y = geometry.y() as usize;

                            let ptr_x = pointer.root_x() as usize;
                            let ptr_y = pointer.root_y() as usize;

                            // XXX include border width
                            let new_width = ptr_x - win_x;
                            let new_height = ptr_y - win_y;

                            if new_width >= 32 && new_height >= 32 {
                                debug!("resizing window: {},{}", new_width, new_height);

                                conn.send_request_checked(&x::ConfigureWindow {
                                    window: win,
                                    value_list: &[
                                        x::ConfigWindow::Width(new_width as u32),
                                        x::ConfigWindow::Height(new_height as u32),
                                    ],
                                });
                                conn.flush()?;
                            }
                        },
                    }
                }
            },

            e => {
                debug!("UNHANDLED: {:?}", e);
            }
        }
    }
}