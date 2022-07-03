use xcb::{x, Connection, Xid};
use log::debug;
use std::collections::HashMap;

use crate::window::Window;

const BORDER_WIDTH: i32 = 2;

pub struct Manager {
    pub conn: Connection,
    pub screen: x::ScreenBuf,

    pub windows: HashMap<x::Window, Window>,

    drag_state: Option<DragState>,
}

#[derive(Clone, Copy, Debug)]
enum DragButton { Left, Right }

#[derive(Clone, Copy, Debug)]
struct DragState {
    button: DragButton,
    window: x::Window,
    off_x: i16,
    off_y: i16,
}

impl Manager {
    pub fn connect() -> xcb::Result<Manager> {
        // connect to server
        let (conn, scr_num) = xcb::Connection::connect(None)?;

        // get screen handle
        let screen = conn.get_setup().roots().nth(scr_num as usize).unwrap().to_owned();

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

        // grab Left (click to raise)
        conn.send_request_checked(&x::GrabButton {
            owner_events: false,
            grab_window: screen.root(),
            event_mask: x::EventMask::BUTTON_PRESS,
            pointer_mode: x::GrabMode::Async,
            keyboard_mode: x::GrabMode::Async,
            confine_to: screen.root(),
            cursor: x::CURSOR_NONE,
            button: x::ButtonIndex::N1,
            modifiers: x::ModMask::empty(),
        });

        // grab Mod4+Left (drag to move)
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

        // grab Mod4+Right (drag to resize)
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

        Ok(Manager {
            conn,
            screen,
            windows: HashMap::default(),
            drag_state: None,
        })
    }

    pub fn attach_existing_windows(&mut self) -> xcb::Result<()> {
        self.windows.clear();

        let tree = self.conn.wait_for_reply(self.conn.send_request(&x::QueryTree {
            window: self.screen.root(),
        }))?;

        tree.children().iter().for_each(|&w| {
            let attr_cookie = self.conn.send_request(&x::GetWindowAttributes {
                window: w,
            });

            let do_map = match self.conn.wait_for_reply(attr_cookie) {
                Err(e) => {
                    debug!("couldn't get window attrs: {:?}", e);
                    false
                },
                Ok(attrs) => {
                    debug!("existing window {:?}, attrs {:?}", w, attrs);

                    // map the window if its not already mapped and not managed by someone else
                    attrs.map_state() != x::MapState::Unmapped && !attrs.override_redirect()
                },
            };

            if do_map {
                debug!("mapping existing window {:?}", w);

                self.windows.insert(w, Window {
                    x_window: w,
                });

                self.map_window(w);
            }
        });

        self.conn.flush()?;

        Ok(())
    }

    pub fn run(&mut self) -> xcb::Result<()> {
        loop {
            match self.conn.wait_for_event()? {

                // new client, just track it
                xcb::Event::X(x::Event::CreateNotify(ev)) => {
                    debug!("new window: {:?}", ev.window());

                    self.windows.insert(ev.window(), Window {
                        x_window: ev.window(),
                    });
                },

                // window gone, forget it
                xcb::Event::X(x::Event::DestroyNotify(ev)) => {
                    debug!("window destroyed: {:?}", ev.window());

                    self.windows.remove(&ev.window());
                }

                // client wants to be displayed
                xcb::Event::X(x::Event::MapRequest(ev)) => {
                    self.map_window(ev.window());
                    self.conn.flush()?;
                },

                // left button inside window area
                xcb::Event::X(x::Event::ButtonPress(ev)) if ev.state().is_empty() => {
                    // ignore if we're not over a window
                    if ev.child().is_none() {
                        continue;
                    }

                    // bring window to front
                    self.bring_window_to_front(ev.child());
                    self.conn.flush()?;
                },

                // Mod4+button inside window area
                xcb::Event::X(x::Event::ButtonPress(ev)) => {
                    // ignore if we're not over a window
                    if ev.child().is_none() {
                        continue;
                    }

                    // bring window to front
                    self.bring_window_to_front(ev.child());

                    // grab the pointer for window move
                    self.conn.send_request(&x::GrabPointer {
                        owner_events: false,
                        grab_window: self.screen.root(),
                        event_mask: x::EventMask::BUTTON_RELEASE | x::EventMask::BUTTON_MOTION | x::EventMask::POINTER_MOTION_HINT,
                        pointer_mode: x::GrabMode::Async,
                        keyboard_mode: x::GrabMode::Async,
                        confine_to: self.screen.root(),
                        cursor: x::CURSOR_NONE,
                        time: x::CURRENT_TIME,
                    });

                    // will need window geometry to compute drag offset
                    let geometry = self.conn.wait_for_reply(self.conn.send_request(&x::GetGeometry {
                        drawable: x::Drawable::Window(ev.child()),
                    }))?;
                    let off_x = ev.root_x() - geometry.x();
                    let off_y = ev.root_y() - geometry.y();

                    // record window
                    self.drag_state = match ev.detail() {
                        1 => Some(DragState {
                            button: DragButton::Left,
                            window: ev.child(),
                            off_x,
                            off_y,
                        }),
                        3 => Some(DragState {
                            button: DragButton::Right,
                            window: ev.child(),
                            off_x,
                            off_y,
                        }),
                        _ => None,
                    };

                    debug!("button down on {:?}, drag state {:?}", ev.child(), self.drag_state);
                },

                xcb::Event::X(x::Event::ButtonRelease(ev)) => {
                    // just release the pointer
                    self.conn.send_request_checked(&x::UngrabPointer {
                        time: x::CURRENT_TIME,
                    });
                    self.conn.flush()?;

                    self.drag_state = None;

                    debug!("button release on {:?}, drag cleared", ev.child());
                },

                xcb::Event::X(x::Event::MotionNotify(_)) => {
                    if let Some(drag_state) = self.drag_state {
                        let pointer = self.conn.wait_for_reply(self.conn.send_request(&x::QueryPointer {
                            window: self.screen.root(),
                        }))?;
                        let geometry = self.conn.wait_for_reply(self.conn.send_request(&x::GetGeometry {
                            drawable: x::Drawable::Window(drag_state.window),
                        }))?;

                        match drag_state.button {
                            DragButton::Left => {

                                let win_width = geometry.width() as i32 + 2*BORDER_WIDTH;
                                let win_height = geometry.height() as i32 + 2*BORDER_WIDTH;

                                let scr_width = self.screen.width_in_pixels() as i32;
                                let scr_height = self.screen.height_in_pixels() as i32;

                                let off_x = drag_state.off_x as i32;
                                let off_y = drag_state.off_y as i32;

                                let ptr_x = pointer.root_x() as i32 - off_x;
                                let ptr_y = pointer.root_y() as i32 - off_y;

                                let new_x = if ptr_x <= 0 {
                                    0
                                } else if ptr_x + win_width > scr_width {
                                    scr_width - win_width
                                } else {
                                    ptr_x
                                };
                                let new_y = if ptr_y <= 0 {
                                    0
                                } else if ptr_y + win_height > scr_height {
                                    scr_height - win_height
                                } else {
                                    ptr_y
                                };

                                debug!("moving {:?} to {},{}", drag_state.window, new_x, new_y);

                                self.conn.send_request_checked(&x::ConfigureWindow {
                                    window: drag_state.window,
                                    value_list: &[
                                        x::ConfigWindow::X(new_x),
                                        x::ConfigWindow::Y(new_y),
                                    ],
                                });
                                self.conn.flush()?;
                            },

                            DragButton::Right => {

                                let win_x = geometry.x() as i32;
                                let win_y = geometry.y() as i32;

                                let ptr_x = pointer.root_x() as i32;
                                let ptr_y = pointer.root_y() as i32;

                                let new_width = ptr_x - win_x + 1 - BORDER_WIDTH*2;
                                let new_height = ptr_y - win_y + 1 - BORDER_WIDTH*2;

                                if new_width >= 32 && new_height >= 32 {
                                    debug!("resizing {:?} to {}x{}", drag_state.window, new_width, new_height);

                                    self.conn.send_request_checked(&x::ConfigureWindow {
                                        window: drag_state.window,
                                        value_list: &[
                                            x::ConfigWindow::Width(new_width as u32),
                                            x::ConfigWindow::Height(new_height as u32),
                                        ],
                                    });
                                    self.conn.flush()?;
                                }
                            },
                        }
                    }
                },

                xcb::Event::X(x::Event::EnterNotify(ev)) => {
                    debug!("pointer entered {:?}, focusing", ev.event());

                    // focus follows mouse :)
                    self.focus_window(ev.event());
                    self.conn.flush()?;
                },

                xcb::Event::X(x::Event::FocusIn(ev)) => {
                    debug!("{:?} received focus", ev.event());

                    self.conn.send_request_checked(&x::ChangeWindowAttributes {
                        window: ev.event(),
                        value_list: &[
                            x::Cw::BorderPixel(0x0055ff),
                        ],
                    });

                    self.conn.flush()?;
                },

                xcb::Event::X(x::Event::FocusOut(ev)) => {
                    debug!("{:?} lost focus", ev.event());

                    self.conn.send_request_checked(&x::ChangeWindowAttributes {
                        window: ev.event(),
                        value_list: &[
                            x::Cw::BorderPixel(0x000000),
                        ],
                    });

                    self.conn.flush()?;
                },

                // silence debug for ones we aren't interested in
                xcb::Event::X(x::Event::ConfigureRequest(_)) => {},

                xcb::Event::X(x::Event::ConfigureNotify(_)) => {},
                xcb::Event::X(x::Event::MapNotify(_)) => {},
                xcb::Event::X(x::Event::UnmapNotify(_)) => {},
                xcb::Event::X(x::Event::MappingNotify(_)) => {},

                xcb::Event::X(x::Event::ClientMessage(_)) => {},

                e => {
                    debug!("UNHANDLED: {:?}", e);
                }
            }
        }
    }

    fn map_window(&mut self, window: x::Window) {
        // XXX some policy or whatever
        let x = 0;
        let y = 0;
        let w = 640;
        let h = 480;

        debug!("mapping {:?} to {},{} {}x{}", window, x, y, w, h);

        // position and size
        // XXX maybe we should take defaults or preferences from ConfigureRequest
        self.conn.send_request_checked(&x::ConfigureWindow {
            window: window,
            value_list: &[
                x::ConfigWindow::X(x),
                x::ConfigWindow::Y(y),
                x::ConfigWindow::Width(w),
                x::ConfigWindow::Height(h),
                x::ConfigWindow::BorderWidth(BORDER_WIDTH as u32),
            ],
        });

        // request enter and focus events
        self.conn.send_request_checked(&x::ChangeWindowAttributes {
            window: window,
            value_list: &[
                x::Cw::EventMask(
                    x::EventMask::ENTER_WINDOW |
                    x::EventMask::FOCUS_CHANGE
                ),
            ],
        });

        // be visible!
        self.conn.send_request_checked(&x::MapWindow {
            window: window,
        });
    }

    fn bring_window_to_front(&mut self, window: x::Window) {
        self.conn.send_request_checked(&x::ConfigureWindow {
            window: window,
            value_list: &[
                x::ConfigWindow::StackMode(x::StackMode::Above),
            ],
        });
        debug!("raised {:?}", window);
    }

    fn focus_window(&mut self, window: x::Window) {
        self.conn.send_request_checked(&x::SetInputFocus {
            revert_to: x::InputFocus::PointerRoot,
            focus: window,
            time: x::CURRENT_TIME,
        });
        debug!("focused {:?}", window);
    }
}
