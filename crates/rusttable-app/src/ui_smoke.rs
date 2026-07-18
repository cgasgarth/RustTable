use iced_test::Simulator;
use iced_test::core::{Settings, Size};
use rusttable_core::PhotoId;

use crate::app::{Message, Shell, update};
use crate::navigation::NavigationIntent;
use crate::presentation::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
    PresentationText,
};
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

fn presentation_text(value: &str) -> PresentationText {
    PresentationText::new(value).expect("test presentation text is valid")
}

fn four_photo_workspace() -> PhotoWorkspaceViewModel {
    let cards = (1..=4)
        .map(|number| {
            PhotoCardViewModel::new(
                PhotoId::new(number).expect("test photo ID is non-zero"),
                presentation_text(&format!("Photo {number}")),
                Some(presentation_text(&format!("Album {number}"))),
            )
        })
        .collect();
    let details = (1..=4)
        .map(|number| {
            let id = PhotoId::new(number).expect("test photo ID is non-zero");
            PhotoDetailViewModel::new(
                id,
                presentation_text(&format!("Detail {number}")),
                vec![PhotoFactViewModel::new(
                    presentation_text("Camera"),
                    presentation_text(&format!("Camera {number}")),
                )],
            )
        })
        .collect();

    PhotoWorkspaceViewModel::new(cards, details).expect("test workspace is valid")
}

#[test]
fn photo_grid_opens_detail_and_returns() -> Result<(), iced_test::Error> {
    let workspace = four_photo_workspace();
    let mut shell = Shell::with_photo_workspace(workspace.clone());
    let mut simulator = Simulator::with_size(
        Settings::default(),
        Size::new(800.0, 600.0),
        view::view(&shell),
    );

    for number in 1..=4 {
        simulator.find(format!("Photo {number}"))?;
        simulator.find(format!("Album {number}"))?;
    }
    simulator.find("Preview unavailable")?;

    let first = simulator.find("Photo 1")?.bounds();
    let second = simulator.find("Photo 2")?.bounds();
    let third = simulator.find("Photo 3")?.bounds();
    let fourth = simulator.find("Photo 4")?.bounds();
    assert_eq!(first.y.to_bits(), second.y.to_bits());
    assert_eq!(second.y.to_bits(), third.y.to_bits());
    assert!(first.x < second.x);
    assert!(second.x < third.x);
    assert!(fourth.y > first.y);

    simulator.click("Photo 2")?;
    let messages: Vec<_> = simulator.into_messages().collect();
    assert_eq!(
        messages,
        [Message::Navigate(NavigationIntent::ShowPhoto(
            PhotoId::new(2).expect("test photo ID is non-zero"),
        ))]
    );
    for message in messages {
        let _ = update(&mut shell, message);
    }

    let mut simulator = Simulator::with_size(
        Settings::default(),
        Size::new(800.0, 600.0),
        view::view(&shell),
    );
    simulator.find("Photo detail")?;
    simulator.find("Detail 2")?;
    simulator.find("Camera")?;
    simulator.find("Camera 2")?;
    assert!(simulator.find("Detail 1").is_err());
    assert!(simulator.find("Camera 1").is_err());
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
    for number in 1..=4 {
        simulator.find(format!("Photo {number}"))?;
    }
    assert_eq!(shell.photo_workspace(), &workspace);

    Ok(())
}
