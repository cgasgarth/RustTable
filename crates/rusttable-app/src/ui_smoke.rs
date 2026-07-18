use iced_test::Simulator;
use iced_test::core::{Settings, Size};
use rusttable_core::PhotoId;

use crate::app::{Message, Shell, update};
use crate::navigation::NavigationIntent;
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

#[test]
fn shell_navigates_from_photo_detail_to_library() -> Result<(), iced_test::Error> {
    let photo_id = PhotoId::new(42).expect("test photo ID is non-zero");
    let mut shell = Shell::default();
    let _ = update(
        &mut shell,
        Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
    );

    let mut simulator = Simulator::with_size(
        Settings::default(),
        Size::new(800.0, 600.0),
        view::view(&shell),
    );

    simulator.find("Workspace")?;
    simulator.find("Photo detail")?;
    simulator.find(photo_id.to_string().as_str())?;
    simulator.find("Back to library")?;
    simulator.click("Back to library")?;

    let messages: Vec<_> = simulator.into_messages().collect();
    assert_eq!(messages, [Message::Navigate(NavigationIntent::ShowLibrary)]);
    for message in messages {
        let _ = update(&mut shell, message);
    }

    let mut simulator = Simulator::with_size(
        Settings::default(),
        Size::new(800.0, 600.0),
        view::view(&shell),
    );

    simulator.find("Workspace")?;
    simulator.find("Library")?;
    assert!(simulator.find("Photo detail").is_err());
    assert!(simulator.find("Back to library").is_err());

    Ok(())
}
