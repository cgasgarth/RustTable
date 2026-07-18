use iced_test::Simulator;
use iced_test::core::{Settings, Size};

use crate::app::{Message, Shell, update};
use crate::view;

#[test]
fn shell_renders_and_toggles_sidebar() -> Result<(), iced_test::Error> {
    let mut shell = Shell::default();
    let mut simulator = Simulator::with_size(
        Settings::default(),
        Size::new(800.0, 600.0),
        view::view(&shell),
    );

    simulator.find("RustTable")?;
    simulator.find("Sidebar")?;
    simulator.find("Workspace")?;
    simulator.find("Hide sidebar")?;
    simulator.click("Hide sidebar")?;

    let messages: Vec<_> = simulator.into_messages().collect();
    assert_eq!(messages, [Message::ToggleSidebar]);
    for message in messages {
        let _ = update(&mut shell, message);
    }

    let mut simulator = Simulator::with_size(
        Settings::default(),
        Size::new(800.0, 600.0),
        view::view(&shell),
    );

    simulator.find("RustTable")?;
    assert!(simulator.find("Sidebar").is_err());
    simulator.find("Workspace")?;
    simulator.find("Show sidebar")?;
    simulator.click("Show sidebar")?;

    let messages: Vec<_> = simulator.into_messages().collect();
    assert_eq!(messages, [Message::ToggleSidebar]);
    for message in messages {
        let _ = update(&mut shell, message);
    }

    let mut simulator = Simulator::with_size(
        Settings::default(),
        Size::new(800.0, 600.0),
        view::view(&shell),
    );

    simulator.find("RustTable")?;
    simulator.find("Sidebar")?;
    simulator.find("Workspace")?;
    simulator.find("Hide sidebar")?;

    Ok(())
}
