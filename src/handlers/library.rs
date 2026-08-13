use super::{
  super::app::{visible_library_options, ActiveBlock, App, RouteId},
  common_key_events,
};
use crate::event::Key;
use crate::backend::IoEvent;

pub fn handler(key: Key, app: &mut App) {
  let options = visible_library_options(&app.hidden_library_sections);
  match key {
    k if common_key_events::right_event(k) => common_key_events::handle_right_event(app),
    k if common_key_events::down_event(k) => {
      let next_index =
        common_key_events::on_down_press_handler(&options, Some(app.library.selected_index));
      app.library.selected_index = next_index;
    }
    k if common_key_events::up_event(k) => {
      let next_index =
        common_key_events::on_up_press_handler(&options, Some(app.library.selected_index));
      app.library.selected_index = next_index;
    }
    k if common_key_events::high_event(k) => {
      let next_index = common_key_events::on_high_press_handler();
      app.library.selected_index = next_index;
    }
    k if common_key_events::middle_event(k) => {
      let next_index = common_key_events::on_middle_press_handler(&options);
      app.library.selected_index = next_index;
    }
    k if common_key_events::low_event(k) => {
      let next_index = common_key_events::on_low_press_handler(&options);
      app.library.selected_index = next_index
    }
    // `library` should probably be an array of structs with enums rather than just using indexes
    // like this
    Key::Char('r') => app.dispatch(IoEvent::RefreshUser),
    Key::Enter => match options
      .get(app.library.selected_index)
      .copied()
      .unwrap_or("")
    {
      // Made For You,
      "For you" => {
        app.push_navigation_stack(RouteId::MadeForYou, ActiveBlock::MadeForYou);
      }
      // Recently Played,
      "Recently Played" => {
        app.dispatch(IoEvent::GetRecentlyPlayed);
        app.push_navigation_stack(RouteId::RecentlyPlayed, ActiveBlock::RecentlyPlayed);
      }
      // Liked Songs,
      "Liked Songs" => {
        app.dispatch(IoEvent::GetCurrentSavedTracks(None));
        app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
      }
      // Albums,
      "Albums" => {
        app.dispatch(IoEvent::GetCurrentUserSavedAlbums(None));
        app.push_navigation_stack(RouteId::AlbumList, ActiveBlock::AlbumList);
      }
      //  Artists,
      "Artists" => {
        app.dispatch(IoEvent::GetFollowedArtists(None));
        app.push_navigation_stack(RouteId::Artists, ActiveBlock::Artists);
      }
      // Podcasts,
      "Podcasts" => {
        app.dispatch(IoEvent::GetCurrentUserSavedShows(None));
        app.push_navigation_stack(RouteId::Podcasts, ActiveBlock::Podcasts);
      }
      // This is required because Rust can't tell if this pattern in exhaustive
      _ => {}
    },
    _ => (),
  };
}
