//! A column that displays the hierarchical tree of subspaces within
//! the currently-selected top-level space.
//!
//! This widget sits to the left of the rooms list and shows only
//! subspaces (no rooms).  Clicking a subspace selects it — causing the
//! rooms list to filter to that subspace's immediate rooms — and
//! toggles expand/collapse to reveal nested sub-subspaces.
//!
//! The widget is hidden when no space is selected, or when the
//! selected space has no subspaces.

use std::collections::{HashMap, HashSet};
use makepad_widgets::*;
use matrix_sdk::ruma::OwnedRoomId;
use crate::{
    home::{
        navigation_tab_bar::{NavigationBarAction, SelectedTab},
        rooms_list::RoomsListRef,
    },
    shared::avatar::{AvatarWidgetExt as _, AvatarWidgetRefExt as _},
    space_service_sync::{SpaceRoomListAction, SpaceRequest, SubspaceDisplayInfo},
    utils,
};


live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    use crate::shared::styles::*;
    use crate::shared::helpers::*;
    use crate::shared::avatar::Avatar;
    use crate::home::space_lobby::TreeLines;

    ICON_COLLAPSE = dep("crate://self/resources/icons/triangle_fill.svg")

    pub NavigatorEntry = {{NavigatorEntry}}<View> {
        width: Fill, height: 36
        flow: Right, align: {y: 0.5}
        padding: {left: 4, right: 4}
        cursor: Hand
        show_bg: true
        draw_bg: {
            instance hover: 0.0
            instance selected: 0.0
            color: #0000
            uniform color_hover: #f0f0f0
            uniform color_selected: #e0e8f0
            fn pixel(self) -> vec4 {
                return mix(
                    mix(self.color, self.color_hover, self.hover),
                    self.color_selected,
                    self.selected
                );
            }
        }

        tree_lines = <TreeLines> {}

        expand_icon = <IconRotated> {
            width: 14, height: 14,
            margin: { left: -6, right: 2 }
            draw_icon: {
                svg_file: (ICON_COLLAPSE)
                rotation_angle: 90.0
                color: #888
            }
            icon_walk: { width: 9, height: 9 }
        }

        avatar = <Avatar> {
            width: 24, height: 24
            margin: {right: 6}
            text_view = { text = { draw_text: {
                text_style: { font_size: 9.0 }
            }}}
        }

        content = <View> {
            width: Fill, height: Fit, flow: Down, spacing: 2,
            name_label = <Label> {
                width: Fill, height: Fit,
                draw_text: {
                    text_style: <REGULAR_TEXT>{font_size: 9.0},
                    color: #1a1a1a,
                    wrap: Ellipsis,
                }
            }
            info_label = <Label> {
                width: Fill, height: Fit,
                draw_text: {
                    text_style: <REGULAR_TEXT>{font_size: 7.5},
                    color: #737373,
                    wrap: Ellipsis,
                }
            }
        }
    }

    pub TopSpaceEntryWidget = {{TopSpaceEntryWidget}}<View> {
        width: Fill, height: 32
        flow: Right, align: {y: 0.5}
        padding: {left: 6, right: 4}
        margin: {bottom: 4}
        cursor: Hand
        show_bg: true
        draw_bg: {
            instance selected: 0.0
            color: #0000
            uniform color_selected: #d8e0ea
            border_radius: 4.0
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                sdf.box(0., 0., self.rect_size.x, self.rect_size.y, self.border_radius);
                sdf.fill(mix(self.color, self.color_selected, self.selected));
                return sdf.result;
            }
        }

        avatar = <Avatar> {
            width: 22, height: 22
            margin: {right: 6}
            text_view = { text = { draw_text: {
                text_style: { font_size: 9.0 }
            }}}
        }

        name_label = <Label> {
            width: Fill, height: Fit,
            draw_text: {
                text_style: <REGULAR_TEXT>{font_size: 9.5},
                color: #1a1a1a,
                wrap: Ellipsis,
            }
        }
    }

    pub SpaceNavigator = {{SpaceNavigator}} {
        visible: false
        width: 170, height: Fill
        flow: Down
        show_bg: true
        draw_bg: {
            color: #f8f8fa
        }

        top_space_entry = <TopSpaceEntryWidget> {}

        list = <PortalList> {
            keep_invisible: false,
            auto_tail: false,
            width: Fill, height: Fill
            flow: Down, spacing: 0

            navigator_entry = <NavigatorEntry> {}
            empty = <View> {}
        }
    }
}


/// An entry in the flattened tree of subspaces for PortalList rendering.
struct NavigatorTreeEntry {
    room_id: OwnedRoomId,
    display_name: String,
    children_count: u64,
    level: usize,
    is_last: bool,
    parent_mask: u32,
}

/// Actions emitted by clicking a navigator entry (subspace) in the tree.
#[derive(Debug, Clone, DefaultNone)]
pub enum NavigatorEntryAction {
    Clicked { room_id: OwnedRoomId },
    None,
}

/// Actions emitted by clicking the top-space entry.
#[derive(Debug, Clone, DefaultNone)]
pub enum TopSpaceEntryAction {
    Clicked,
    None,
}

/// Actions emitted by the SpaceNavigator widget to the rest of the app.
#[derive(Debug, Clone, DefaultNone)]
pub enum SpaceNavigatorAction {
    /// A subspace was selected or deselected.
    /// `None` means back to the top-level space (no subspace selected).
    SubspaceSelected { subspace_id: Option<OwnedRoomId> },
    None,
}


// ---- Inner widget: NavigatorEntry (clickable subspace row) ----

#[derive(Live, LiveHook, Widget)]
pub struct NavigatorEntry {
    #[deref] view: View,
    /// The room ID of the subspace this entry represents.
    #[rust] pub room_id: Option<OwnedRoomId>,
}

impl Widget for NavigatorEntry {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        match event.hits(cx, self.view.area()) {
            Hit::FingerUp(fe) if fe.is_over && fe.is_primary_hit() && fe.was_tap() => {
                if let Some(room_id) = self.room_id.clone() {
                    cx.widget_action(
                        self.widget_uid(),
                        &scope.path,
                        NavigatorEntryAction::Clicked { room_id },
                    );
                }
            }
            _ => {}
        }
        self.view.handle_event(cx, event, scope);
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        self.view.draw_walk(cx, scope, walk)
    }
}


// ---- Inner widget: TopSpaceEntryWidget (clickable top-space row) ----

#[derive(Live, LiveHook, Widget)]
pub struct TopSpaceEntryWidget {
    #[deref] view: View,
}

impl Widget for TopSpaceEntryWidget {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        if let Hit::FingerUp(fe) = event.hits(cx, self.view.area()) {
            if fe.is_over && fe.is_primary_hit() && fe.was_tap() {
                cx.widget_action(
                    self.widget_uid(),
                    &scope.path,
                    TopSpaceEntryAction::Clicked,
                );
            }
        }
        self.view.handle_event(cx, event, scope);
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        self.view.draw_walk(cx, scope, walk)
    }
}


// ---- Main widget: SpaceNavigator ----

#[derive(Live, Widget)]
pub struct SpaceNavigator {
    #[deref] view: View,
    /// The top-level space being navigated.
    #[rust] current_top_space: Option<OwnedRoomId>,
    /// Display name of the top-level space.
    #[rust] current_top_space_name: String,
    /// Flat list of tree entries for PortalList rendering.
    #[rust] tree_entries: Vec<NavigatorTreeEntry>,
    /// Which subspaces are expanded in the tree.
    #[rust] expanded_subspaces: HashSet<OwnedRoomId>,
    /// The currently-selected subspace (determines rooms list filter).
    #[rust] selected_subspace: Option<OwnedRoomId>,
    /// Cached subspace info for each space, keyed by space_id.
    /// Each value is the list of direct subspace display infos within that space.
    #[rust] subspace_cache: HashMap<OwnedRoomId, Vec<SubspaceDisplayInfo>>,
}

impl LiveHook for SpaceNavigator {
    fn after_new_from_doc(&mut self, _cx: &mut Cx) { }
}

impl SpaceNavigator {
    /// Rebuild the flattened tree entries from the cached subspace info.
    fn rebuild_tree(&mut self) {
        self.tree_entries.clear();
        let Some(top_space_id) = &self.current_top_space else { return };
        Self::build_tree_recursive(
            &self.subspace_cache,
            &self.expanded_subspaces,
            &mut self.tree_entries,
            top_space_id,
            0,
            0,
        );
    }

    /// Recursively build the flat tree of subspaces for rendering.
    fn build_tree_recursive(
        subspace_cache: &HashMap<OwnedRoomId, Vec<SubspaceDisplayInfo>>,
        expanded_subspaces: &HashSet<OwnedRoomId>,
        tree_entries: &mut Vec<NavigatorTreeEntry>,
        space_id: &OwnedRoomId,
        level: usize,
        parent_mask: u32,
    ) {
        let Some(subspace_infos) = subspace_cache.get(space_id) else { return };

        let count = subspace_infos.len();
        for (i, si) in subspace_infos.iter().enumerate() {
            let is_last = i == count - 1;

            tree_entries.push(NavigatorTreeEntry {
                room_id: si.room_id.clone(),
                display_name: si.display_name.clone(),
                children_count: si.children_count,
                level,
                is_last,
                parent_mask,
            });

            // If this subspace is expanded, recurse into its children.
            if expanded_subspaces.contains(&si.room_id) {
                let child_mask = if is_last {
                    parent_mask
                } else {
                    parent_mask | (1 << level)
                };
                Self::build_tree_recursive(
                    subspace_cache,
                    expanded_subspaces,
                    tree_entries,
                    &si.room_id,
                    level + 1,
                    child_mask,
                );
            }
        }
    }

    /// Returns true if the current top-level space has any subspaces visible.
    fn has_subspaces(&self) -> bool {
        self.current_top_space.as_ref()
            .and_then(|id| self.subspace_cache.get(id))
            .is_some_and(|infos| !infos.is_empty())
    }

    /// Update visibility based on whether the navigator should be shown.
    fn update_visibility(&mut self, cx: &mut Cx) {
        let should_show = self.current_top_space.is_some() && self.has_subspaces();
        self.view.set_visible(cx, should_show);
        if should_show {
            self.redraw(cx);
        }
    }

    /// Ensure a subspace is subscribed/paginated if it hasn't been yet.
    fn ensure_subspace_loaded(&self, cx: &mut Cx, subspace_id: &OwnedRoomId) {
        let rooms_list_ref = cx.get_global::<RoomsListRef>();
        let Some(sender) = rooms_list_ref.get_space_request_sender() else { return };
        let parent_chain = rooms_list_ref.get_space_parent_chain(subspace_id)
            .unwrap_or_default();
        let _ = sender.send(SpaceRequest::SubscribeToSpaceRoomList {
            space_id: subspace_id.clone(),
            parent_chain: parent_chain.clone(),
        });
        let _ = sender.send(SpaceRequest::PaginateSpaceRoomList {
            space_id: subspace_id.clone(),
            parent_chain: parent_chain.clone(),
        });
        let _ = sender.send(SpaceRequest::GetChildren {
            space_id: subspace_id.clone(),
            parent_chain,
        });
    }

    /// Process a click on a navigator entry (subspace).
    fn handle_navigator_entry_click(
        &mut self,
        cx: &mut Cx,
        scope: &mut Scope,
        room_id: &OwnedRoomId,
    ) {
        // Toggle expand/collapse.
        if self.expanded_subspaces.contains(room_id) {
            self.expanded_subspaces.remove(room_id);
        } else {
            self.expanded_subspaces.insert(room_id.clone());
            // Ensure subspace children are loaded.
            if !self.subspace_cache.contains_key(room_id) {
                self.ensure_subspace_loaded(cx, room_id);
            }
        }

        // Select this subspace.
        self.selected_subspace = Some(room_id.clone());
        cx.widget_action(
            self.widget_uid(),
            &scope.path,
            SpaceNavigatorAction::SubspaceSelected {
                subspace_id: Some(room_id.clone()),
            },
        );

        self.rebuild_tree();
        self.redraw(cx);
    }
}


impl Widget for SpaceNavigator {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        // Forward events to inner views (this propagates to the PortalList
        // and its item widgets, including NavigatorEntry and TopSpaceEntryWidget).
        let navigator_actions = cx.capture_actions(
            |cx| self.view.handle_event(cx, event, scope)
        );

        // Process actions emitted by inner widgets.
        for action in &navigator_actions {
            // Handle navigator entry clicks.
            if let NavigatorEntryAction::Clicked { room_id }
                = action.as_widget_action().cast()
            {
                self.handle_navigator_entry_click(cx, scope, &room_id);
                continue;
            }

            // Handle top-space entry click.
            if let TopSpaceEntryAction::Clicked = action.as_widget_action().cast() {
                self.selected_subspace = None;
                cx.widget_action(
                    self.widget_uid(),
                    &scope.path,
                    SpaceNavigatorAction::SubspaceSelected { subspace_id: None },
                );
                self.redraw(cx);
                continue;
            }
        }

        // Handle global actions (space tab changes, space room list updates).
        if let Event::Actions(actions) = event {
            for action in actions {
                // Handle space tab selection from the navigation bar.
                if let Some(NavigationBarAction::TabSelected(tab)) = action.downcast_ref() {
                    match tab {
                        SelectedTab::Space { space_name_id } => {
                            let new_space_id = space_name_id.room_id().clone();
                            if self.current_top_space.as_ref() != Some(&new_space_id) {
                                self.current_top_space = Some(new_space_id);
                                self.current_top_space_name =
                                    space_name_id.display_name().to_string();
                                self.expanded_subspaces.clear();
                                self.selected_subspace = None;
                                self.rebuild_tree();
                                self.update_visibility(cx);
                            }
                        }
                        _ => {
                            self.current_top_space = None;
                            self.current_top_space_name.clear();
                            self.expanded_subspaces.clear();
                            self.selected_subspace = None;
                            self.tree_entries.clear();
                            self.view.set_visible(cx, false);
                        }
                    }
                    continue;
                }

                // Handle space room list updates (children changed).
                if let Some(SpaceRoomListAction::UpdatedChildren {
                    space_id, subspace_infos, ..
                }) = action.downcast_ref() {
                    // Only process if this space is relevant to our hierarchy.
                    let dominated = self.current_top_space.as_ref().is_some_and(|top_id| {
                        top_id == space_id
                        || self.subspace_cache.contains_key(space_id)
                        || self.expanded_subspaces.contains(space_id)
                    });
                    if dominated {
                        self.subspace_cache.insert(
                            space_id.clone(),
                            subspace_infos.as_ref().clone(),
                        );
                        self.rebuild_tree();
                        self.update_visibility(cx);
                    }
                    continue;
                }
            }
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        // Set the top-space entry selected state.
        let is_top_selected = self.selected_subspace.is_none();
        self.view(ids!(top_space_entry)).apply_over(cx, live! {
            draw_bg: { selected: (if is_top_selected { 1.0 } else { 0.0 }) }
        });

        // Set the top-space name and avatar.
        if !self.current_top_space_name.is_empty() {
            self.view.label(ids!(top_space_entry.name_label))
                .set_text(cx, &self.current_top_space_name);
            let first_char = utils::user_name_first_letter(&self.current_top_space_name);
            self.view.avatar(ids!(top_space_entry.avatar))
                .show_text(cx, None, None, first_char.unwrap_or("#"));
        }

        // Draw the PortalList entries.
        let total_count = self.tree_entries.len();

        while let Some(widget_to_draw) = self.view.draw_walk(cx, scope, walk).step() {
            let portal_list_ref = widget_to_draw.as_portal_list();
            let Some(mut list) = portal_list_ref.borrow_mut() else { continue };

            list.set_item_range(cx, 0, total_count);

            while let Some(item_id) = list.next_visible_item(cx) {
                let item = if let Some(entry) = self.tree_entries.get(item_id) {
                    let item = list.item(cx, item_id, id!(navigator_entry));

                    // Set the room_id on the inner NavigatorEntry widget.
                    if let Some(mut inner) = item.borrow_mut::<NavigatorEntry>() {
                        inner.room_id = Some(entry.room_id.clone());
                    }

                    // Set tree lines.
                    if let Some(mut lines) = item.widget(ids!(tree_lines))
                        .borrow_mut::<crate::home::space_lobby::TreeLines>()
                    {
                        lines.set_properties(
                            entry.level as f32,
                            if entry.is_last { 1.0 } else { 0.0 },
                            entry.parent_mask as f32,
                            30.0,
                        );
                    }

                    // Expand icon.
                    let is_expanded = self.expanded_subspaces.contains(&entry.room_id);
                    let has_children = entry.children_count > 0
                        || self.subspace_cache.get(&entry.room_id)
                            .is_some_and(|v| !v.is_empty());
                    let angle = if is_expanded { 180.0 } else { 90.0 };
                    item.icon(ids!(expand_icon)).apply_over(cx, live! {
                        draw_icon: { rotation_angle: (angle) }
                    });
                    item.icon(ids!(expand_icon)).set_visible(cx, has_children);

                    // Avatar.
                    let first_char = utils::user_name_first_letter(&entry.display_name);
                    item.avatar(ids!(avatar))
                        .show_text(cx, None, None, first_char.unwrap_or("#"));

                    // Labels.
                    item.label(ids!(content.name_label))
                        .set_text(cx, &entry.display_name);
                    let info_text = if entry.children_count == 1 {
                        "1 child".to_string()
                    } else if entry.children_count > 1 {
                        format!("{} children", entry.children_count)
                    } else {
                        String::new()
                    };
                    item.label(ids!(content.info_label)).set_text(cx, &info_text);

                    // Selection highlight.
                    let is_selected =
                        self.selected_subspace.as_ref() == Some(&entry.room_id);
                    item.apply_over(cx, live! {
                        draw_bg: {
                            selected: (if is_selected { 1.0 } else { 0.0 })
                        }
                    });

                    item
                } else {
                    list.item(cx, item_id, id!(empty))
                };

                let mut scope = Scope::empty();
                item.draw_all(cx, &mut scope);
            }
        }

        DrawStep::done()
    }
}
