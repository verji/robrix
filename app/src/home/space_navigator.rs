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
//!
//! NOTE: We deliberately avoid using a `PortalList` here because Makepad's
//! PortalList corrupts the draw list when placed inside a `visible: false`
//! parent, causing the entire app to freeze. Instead we dynamically create
//! widget instances from a LivePtr template and draw them using the
//! begin_turtle/end_turtle manual layout pattern.

use std::collections::{HashMap, HashSet};
use makepad_widgets::*;
use matrix_sdk::ruma::OwnedRoomId;
use crate::{
    home::{
        navigation_tab_bar::{NavigationBarAction, SelectedTab},
        rooms_list::RoomsListRef,
    },
    shared::avatar::AvatarWidgetRefExt as _,
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

    ICON_COLLAPSE = dep("crate://self/resources/icons/triangle_fill.svg")

    NavigatorEntryTemplate = <View> {
        width: Fill, height: 36
        flow: Right, align: {y: 0.5}
        padding: {left: 4, right: 4}
        cursor: Hand
        show_bg: true
        draw_bg: {
            instance selected: 0.0
            color: #0000
            uniform color_selected: #e0e8f0
            fn pixel(self) -> vec4 {
                return mix(self.color, self.color_selected, self.selected);
            }
        }

        expand_icon = <Label> {
            width: 14, height: Fill,
            align: {x: 0.5, y: 0.5}
            draw_text: {
                text_style: <REGULAR_TEXT>{font_size: 10.0},
                color: #888,
            }
            text: "▸"
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

    TopSpaceEntryTemplate = <View> {
        width: Fill, height: 32
        flow: Right, align: {y: 0.5}
        padding: {left: 6, right: 4}
        margin: {bottom: 4}
        cursor: Hand
        show_bg: true
        draw_bg: {
            instance selected: 0.0
            instance border_radius: 4.0
            color: #0000
            uniform color_selected: #d8e0ea
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
        draw_bg: {
            color: #f8f8fa
        }
        top_space_entry: <TopSpaceEntryTemplate> {}
        navigator_entry_template: <NavigatorEntryTemplate> {}
    }
}


/// An entry in the flattened tree of subspaces.
struct NavigatorTreeEntry {
    room_id: OwnedRoomId,
    display_name: String,
    children_count: u64,
    level: usize,
}

/// A rendered widget instance for a tree entry.
struct RenderedEntry {
    widget: WidgetRef,
    room_id: OwnedRoomId,
}

/// Actions emitted by the SpaceNavigator widget to the rest of the app.
#[derive(Debug, Clone, DefaultNone)]
pub enum SpaceNavigatorAction {
    /// A subspace was selected or deselected.
    /// `None` means back to the top-level space (no subspace selected).
    SubspaceSelected { subspace_id: Option<OwnedRoomId> },
    None,
}


const INDENT_PER_LEVEL: f64 = 16.0;

#[derive(Live)]
pub struct SpaceNavigator {
    #[layout] layout: Layout,
    #[walk] walk: Walk,
    #[live] draw_bg: DrawQuad,
    #[live] visible: bool,

    /// Template for the top-space entry widget.
    #[live] top_space_entry: Option<LivePtr>,
    /// Template for navigator entry widgets.
    #[live] navigator_entry_template: Option<LivePtr>,

    /// The created top-space entry widget.
    #[rust] top_space_widget: Option<WidgetRef>,
    /// Rendered widget instances for tree entries.
    #[rust] rendered_entries: Vec<RenderedEntry>,

    /// The top-level space being navigated.
    #[rust] current_top_space: Option<OwnedRoomId>,
    /// Display name of the top-level space.
    #[rust] current_top_space_name: String,
    /// Flat list of tree entries (logical model).
    #[rust] tree_entries: Vec<NavigatorTreeEntry>,
    /// Which subspaces are expanded in the tree.
    #[rust] expanded_subspaces: HashSet<OwnedRoomId>,
    /// The currently-selected subspace (determines rooms list filter).
    #[rust] selected_subspace: Option<OwnedRoomId>,
    /// Cached subspace info for each space, keyed by space_id.
    #[rust] subspace_cache: HashMap<OwnedRoomId, Vec<SubspaceDisplayInfo>>,
}

impl LiveRegister for SpaceNavigator {
    fn live_register(cx: &mut Cx) {
        register_widget!(cx, SpaceNavigator);
    }
}

impl LiveHook for SpaceNavigator {
    fn after_new_from_doc(&mut self, cx: &mut Cx) {
        // Create the top-space entry widget from its template.
        self.top_space_widget = Some(WidgetRef::new_from_ptr(cx, self.top_space_entry));
    }
}

impl SpaceNavigator {
    fn rebuild_tree(&mut self) {
        self.tree_entries.clear();
        let Some(top_space_id) = &self.current_top_space else { return };
        Self::build_tree_recursive(
            &self.subspace_cache,
            &self.expanded_subspaces,
            &mut self.tree_entries,
            top_space_id,
            0,
        );
    }

    fn build_tree_recursive(
        subspace_cache: &HashMap<OwnedRoomId, Vec<SubspaceDisplayInfo>>,
        expanded_subspaces: &HashSet<OwnedRoomId>,
        tree_entries: &mut Vec<NavigatorTreeEntry>,
        space_id: &OwnedRoomId,
        level: usize,
    ) {
        let Some(subspace_infos) = subspace_cache.get(space_id) else { return };

        for si in subspace_infos.iter() {
            tree_entries.push(NavigatorTreeEntry {
                room_id: si.room_id.clone(),
                display_name: si.display_name.clone(),
                children_count: si.children_count,
                level,
            });

            if expanded_subspaces.contains(&si.room_id) {
                Self::build_tree_recursive(
                    subspace_cache,
                    expanded_subspaces,
                    tree_entries,
                    &si.room_id,
                    level + 1,
                );
            }
        }
    }

    fn has_subspaces(&self) -> bool {
        self.current_top_space.as_ref()
            .and_then(|id| self.subspace_cache.get(id))
            .is_some_and(|infos| !infos.is_empty())
    }

    fn update_visibility(&mut self, cx: &mut Cx) {
        let should_show = self.current_top_space.is_some() && self.has_subspaces();
        self.visible = should_show;
        self.draw_bg.redraw(cx);
    }

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

    fn handle_navigator_entry_click(
        &mut self,
        cx: &mut Cx,
        scope: &mut Scope,
        room_id: &OwnedRoomId,
    ) {
        if self.expanded_subspaces.contains(room_id) {
            self.expanded_subspaces.remove(room_id);
        } else {
            self.expanded_subspaces.insert(room_id.clone());
            if !self.subspace_cache.contains_key(room_id) {
                self.ensure_subspace_loaded(cx, room_id);
            }
        }

        self.selected_subspace = Some(room_id.clone());
        cx.widget_action(
            self.widget_uid(),
            &scope.path,
            SpaceNavigatorAction::SubspaceSelected {
                subspace_id: Some(room_id.clone()),
            },
        );

        self.rebuild_tree();
        self.draw_bg.redraw(cx);
    }

    /// Ensure we have enough rendered entry widgets, creating from template as needed.
    fn ensure_entry_widgets(&mut self, cx: &mut Cx) {
        let needed = self.tree_entries.len();

        // Truncate excess.
        self.rendered_entries.truncate(needed);

        // Create new widgets as needed.
        let current = self.rendered_entries.len();
        for i in current..needed {
            let widget = WidgetRef::new_from_ptr(cx, self.navigator_entry_template);
            self.rendered_entries.push(RenderedEntry {
                widget,
                room_id: self.tree_entries[i].room_id.clone(),
            });
        }

        // Update room_ids for reused entries.
        for (i, entry) in self.tree_entries.iter().enumerate() {
            if i < current {
                self.rendered_entries[i].room_id = entry.room_id.clone();
            }
        }
    }

    /// Handle global actions (space tab changes, space room list updates).
    /// Called even when the navigator is not visible.
    fn handle_global_actions(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        let _ = scope;
        if let Event::Actions(actions) = event {
            for action in actions {
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
                            self.rendered_entries.clear();
                            self.visible = false;
                            self.draw_bg.redraw(cx);
                        }
                    }
                    continue;
                }

                if let Some(SpaceRoomListAction::UpdatedChildren {
                    space_id, subspace_infos, ..
                }) = action.downcast_ref() {
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
}


impl WidgetNode for SpaceNavigator {
    fn walk(&mut self, _cx: &mut Cx) -> Walk { self.walk }
    fn area(&self) -> Area { self.draw_bg.area() }
    fn redraw(&mut self, cx: &mut Cx) { self.draw_bg.redraw(cx); }
    fn uid_to_widget(&self, _uid: WidgetUid) -> WidgetRef { WidgetRef::empty() }
    fn find_widgets(&self, _path: &[LiveId], _cached: WidgetCache, _results: &mut WidgetSet) {}
}

impl Widget for SpaceNavigator {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, scope: &mut Scope) {
        // Always handle global actions (even when not visible),
        // since these control whether the navigator should become visible.
        self.handle_global_actions(cx, event, scope);

        if !self.visible { return; }

        // Forward events to top-space entry widget.
        if let Some(top_widget) = &mut self.top_space_widget {
            top_widget.handle_event(cx, event, scope);
            // Check for top-space click.
            if let Hit::FingerUp(fe) = event.hits(cx, top_widget.area()) {
                if fe.is_over && fe.is_primary_hit() && fe.was_tap() {
                    self.selected_subspace = None;
                    cx.widget_action(
                        self.widget_uid(),
                        &scope.path,
                        SpaceNavigatorAction::SubspaceSelected { subspace_id: None },
                    );
                    self.draw_bg.redraw(cx);
                }
            }
        }

        // Forward events to and check clicks on tree entry widgets.
        for rendered in &self.rendered_entries {
            rendered.widget.handle_event(cx, event, scope);
            if let Hit::FingerUp(fe) = event.hits(cx, rendered.widget.area()) {
                if fe.is_over && fe.is_primary_hit() && fe.was_tap() {
                    let room_id = rendered.room_id.clone();
                    self.handle_navigator_entry_click(cx, scope, &room_id);
                    break;
                }
            }
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, scope: &mut Scope, walk: Walk) -> DrawStep {
        if !self.visible {
            return DrawStep::done();
        }

        // Ensure entry widgets exist.
        self.ensure_entry_widgets(cx);

        // Begin the background + layout.
        self.draw_bg.begin(cx, walk, self.layout);

        // Draw the top-space entry.
        if let Some(top_widget) = &mut self.top_space_widget {
            // Configure top entry.
            let is_top_selected = self.selected_subspace.is_none();
            top_widget.apply_over(cx, live! {
                draw_bg: { selected: (if is_top_selected { 1.0 } else { 0.0 }) }
            });
            if !self.current_top_space_name.is_empty() {
                top_widget.label(ids!(name_label))
                    .set_text(cx, &self.current_top_space_name);
                let first_char = utils::user_name_first_letter(&self.current_top_space_name);
                top_widget.avatar(ids!(avatar))
                    .show_text(cx, None, None, first_char.unwrap_or("#"));
            }
            let _ = top_widget.draw(cx, scope);
        }

        // Draw each tree entry widget.
        for (i, entry) in self.tree_entries.iter().enumerate() {
            let Some(rendered) = self.rendered_entries.get_mut(i) else { break };
            let w = &mut rendered.widget;

            let indent = 4.0 + (entry.level as f64) * INDENT_PER_LEVEL;
            w.apply_over(cx, live! {
                padding: { left: (indent) }
            });

            // Expand icon.
            let is_expanded = self.expanded_subspaces.contains(&entry.room_id);
            let has_children = entry.children_count > 0
                || self.subspace_cache.get(&entry.room_id)
                    .is_some_and(|v| !v.is_empty());
            let icon_text = if !has_children {
                "  "
            } else if is_expanded {
                "▾"
            } else {
                "▸"
            };
            w.label(ids!(expand_icon)).set_text(cx, icon_text);

            // Avatar.
            let first_char = utils::user_name_first_letter(&entry.display_name);
            w.avatar(ids!(avatar))
                .show_text(cx, None, None, first_char.unwrap_or("#"));

            // Labels.
            w.label(ids!(content.name_label)).set_text(cx, &entry.display_name);
            let info_text = if entry.children_count == 1 {
                "1 child".to_string()
            } else if entry.children_count > 1 {
                format!("{} children", entry.children_count)
            } else {
                String::new()
            };
            w.label(ids!(content.info_label)).set_text(cx, &info_text);

            // Selection highlight.
            let is_selected = self.selected_subspace.as_ref() == Some(&entry.room_id);
            w.apply_over(cx, live! {
                draw_bg: {
                    selected: (if is_selected { 1.0 } else { 0.0 })
                }
            });

            let _ = w.draw(cx, scope);
        }

        self.draw_bg.end(cx);
        DrawStep::done()
    }
}
