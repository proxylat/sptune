use super::{
  super::app::{visible_library_options, ActiveBlock, App, RouteId, TrackTableContext},
  common_key_events,
};
use crate::backend::IoEvent;
use crate::event::Key;

pub fn handler(key: Key, app: &mut App) {
  if common_key_events::down_event(key)
    || common_key_events::up_event(key)
    || common_key_events::high_event(key)
    || common_key_events::middle_event(key)
    || common_key_events::low_event(key)
  {
    app.selection_engaged = true;
  }
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
    k if Some(k) == app.user_config.keys.refresh => app.dispatch(IoEvent::RefreshUser),
    Key::Enter => {
      app.clear_search_input();
      match options
        .get(app.library.selected_index)
        .copied()
        .unwrap_or("")
      {
      // Clicking the page we are already on must not re-fetch (or re-stack).
      "For you" => {
        if app.get_current_route().id != RouteId::MadeForYou {
          app.push_navigation_stack(RouteId::MadeForYou, ActiveBlock::MadeForYou);
        }
      }
      // Recently Played,
      "Recently Played" => {
        if app.get_current_route().active_block != ActiveBlock::RecentlyPlayed {
          app.dispatch(IoEvent::GetRecentlyPlayed);
          app.push_navigation_stack(RouteId::RecentlyPlayed, ActiveBlock::RecentlyPlayed);
        }
      }
      // Liked Songs,
      "Liked Songs" => {
        if app.track_table.context != Some(TrackTableContext::SavedTracks) {
          app.dispatch(IoEvent::GetCurrentSavedTracks(None));
          app.push_navigation_stack(RouteId::TrackTable, ActiveBlock::TrackTable);
        }
      }
      // Albums,
      "Albums" => {
        if app.get_current_route().id != RouteId::AlbumList {
          app.dispatch(IoEvent::GetCurrentUserSavedAlbums(None));
          app.push_navigation_stack(RouteId::AlbumList, ActiveBlock::AlbumList);
        }
      }
      //  Artists,
      "Artists" => {
        if app.get_current_route().id != RouteId::Artists {
          app.dispatch(IoEvent::GetFollowedArtists(None));
          app.push_navigation_stack(RouteId::Artists, ActiveBlock::Artists);
        }
      }
      // Podcasts,
      "Podcasts" => {
        if app.get_current_route().id != RouteId::Podcasts {
          app.dispatch(IoEvent::GetCurrentUserSavedShows(None));
          app.push_navigation_stack(RouteId::Podcasts, ActiveBlock::Podcasts);
        }
      }
      // This is required because Rust can't tell if this pattern in exhaustive
      _ => {}
    }
  },
  _ => (),
  };
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::TrackTableContext;

  fn option_index(option: &str) -> usize {
    visible_library_options(&[])
      .iter()
      .position(|o| *o == option)
      .unwrap()
  }

  fn app_with_route(
    route_id: RouteId,
    block: ActiveBlock,
  ) -> (App, std::sync::mpsc::Receiver<IoEvent>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::default();
    app.io_tx = Some(tx);
    app.push_navigation_stack(route_id, block);
    (app, rx)
  }

  #[test]
  fn sidebar_enter_on_current_page_does_not_refetch() {
    let (mut app, rx) = app_with_route(RouteId::RecentlyPlayed, ActiveBlock::RecentlyPlayed);
    app.library.selected_index = option_index("Recently Played");

    handler(Key::Enter, &mut app);

    assert!(rx.try_iter().collect::<Vec<_>>().is_empty());
    assert_eq!(app.get_current_route().id, RouteId::RecentlyPlayed);
  }

  #[test]
  fn sidebar_enter_on_different_page_dispatches_and_stacks() {
    let (mut app, rx) = app_with_route(RouteId::TrackTable, ActiveBlock::TrackTable);
    app.library.selected_index = option_index("Recently Played");

    handler(Key::Enter, &mut app);

    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(dispatched, vec![IoEvent::GetRecentlyPlayed]);
  }

  #[test]
  fn sidebar_liked_songs_noop_when_already_saved_tracks() {
    let (mut app, rx) = app_with_route(RouteId::TrackTable, ActiveBlock::TrackTable);
    app.track_table.context = Some(TrackTableContext::SavedTracks);
    app.library.selected_index = option_index("Liked Songs");

    handler(Key::Enter, &mut app);

    assert!(rx.try_iter().collect::<Vec<_>>().is_empty());
  }

  #[test]
  fn sidebar_liked_songs_navigates_from_a_playlist() {
    let (mut app, rx) = app_with_route(RouteId::TrackTable, ActiveBlock::TrackTable);
    app.track_table.context = Some(TrackTableContext::MyPlaylists);
    app.library.selected_index = option_index("Liked Songs");

    handler(Key::Enter, &mut app);

    let dispatched: Vec<IoEvent> = rx.try_iter().collect();
    assert_eq!(dispatched, vec![IoEvent::GetCurrentSavedTracks(None)]);
  }
}
