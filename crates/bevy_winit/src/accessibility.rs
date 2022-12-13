use std::{
    collections::VecDeque,
    num::NonZeroU128,
    sync::{Arc, Mutex},
};

use accesskit_winit::Adapter;
use bevy_a11y::accesskit::{ActionHandler, ActionRequest, Node, NodeId, Role, TreeUpdate};
use bevy_app::{App, CoreStage, Plugin};
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::{
    prelude::{Component, Entity, EventReader, EventWriter},
    query::{Changed, With},
    system::{NonSend, NonSendMut, Query, RemovedComponents, Res, ResMut, Resource},
};
use bevy_utils::{default, HashMap};
use bevy_window::{WindowClosed, WindowFocused, WindowId};

#[derive(Component, Clone, Default, Deref, DerefMut)]
pub struct AccessibilityNode(pub Node);

impl From<Node> for AccessibilityNode {
    fn from(node: Node) -> Self {
        Self(node)
    }
}

#[derive(Default, Deref, DerefMut)]
pub struct Adapters(pub HashMap<WindowId, Adapter>);

impl Adapters {
    pub fn get_primary_adapter(&self) -> Option<&Adapter> {
        self.get(&WindowId::primary())
    }
}

#[derive(Resource, Default, Deref, DerefMut)]
pub struct Handlers(pub HashMap<WindowId, WinitActionHandler>);

#[derive(Clone, Default, Deref, DerefMut)]
pub struct WinitActionHandler(pub Arc<Mutex<VecDeque<ActionRequest>>>);

impl ActionHandler for WinitActionHandler {
    fn do_action(&self, request: ActionRequest) {
        println!("Pushing {:?}", request);
        let mut requests = self.0.lock().unwrap();
        requests.push_back(request);
    }
}

pub trait AccessKitEntityExt {
    fn from_node_id(id: &NodeId) -> Entity {
        Entity::from_bits((id.0.get() - 1) as u64)
    }

    fn to_node_id(&self) -> NodeId;
}

impl AccessKitEntityExt for Entity {
    fn to_node_id(&self) -> NodeId {
        let id = NonZeroU128::new((self.to_bits() + 1) as u128);
        NodeId(id.unwrap())
    }
}

#[derive(Resource, Default, Deref, DerefMut)]
pub struct LastFocus(Option<NodeId>);

impl LastFocus {
    pub fn from_entity(&mut self, entity: Option<Entity>) {
        **self = entity.map(|v| v.to_node_id());
    }

    pub fn entity(&self) -> Option<Entity> {
        self.0.as_ref().map(<Entity>::from_node_id)
    }
}

fn handle_focus(
    last_focus: Res<LastFocus>,
    adapters: NonSend<Adapters>,
    mut focus: EventReader<WindowFocused>,
) {
    let focus_id = (*last_focus)
        .unwrap_or_else(|| NodeId(NonZeroU128::new(WindowId::primary().as_u128()).unwrap()));
    for event in focus.iter() {
        if let Some(adapter) = adapters.get_primary_adapter() {
            adapter.update(TreeUpdate {
                focus: if event.focused { Some(focus_id) } else { None },
                ..default()
            });
        }
    }
}

fn window_closed(
    mut adapters: NonSendMut<Adapters>,
    mut receivers: ResMut<Handlers>,
    mut events: EventReader<WindowClosed>,
) {
    for WindowClosed { id, .. } in events.iter() {
        adapters.remove(id);
        receivers.remove(id);
    }
}

fn poll_receivers(handlers: Res<Handlers>, mut actions: EventWriter<ActionRequest>) {
    for (_id, handler) in handlers.iter() {
        let mut handler = handler.lock().unwrap();
        while let Some(event) = handler.pop_front() {
            actions.send(event);
        }
    }
}

fn update_accessibility_nodes(
    adapters: NonSend<Adapters>,
    query: Query<(Entity, &AccessibilityNode), Changed<AccessibilityNode>>,
) {
    let mut nodes = vec![];
    for (entity, node) in &query {
        nodes.push((entity.to_node_id(), Arc::new((**node).clone())));
    }
    if let Some(adapter) = adapters.get_primary_adapter() {
        if !nodes.is_empty() {
            let root_id = NodeId(NonZeroU128::new(WindowId::primary().as_u128()).unwrap());
            let children = nodes.iter().map(|v| v.0).collect::<Vec<NodeId>>();
            let window_update = (
                root_id,
                Arc::new(Node {
                    role: Role::Window,
                    children,
                    ..default()
                }),
            );
            nodes.insert(0, window_update);
            adapter.update(TreeUpdate { nodes, ..default() });
        }
    }
}

fn remove_accessibility_nodes(
    adapters: NonSend<Adapters>,
    mut last_focus: ResMut<LastFocus>,
    removed: RemovedComponents<AccessibilityNode>,
    remaining_nodes: Query<Entity, With<AccessibilityNode>>,
) {
    if removed.iter().len() != 0 {
        if let Some(last_focused_entity) = last_focus.entity() {
            for entity in removed.iter() {
                if entity == last_focused_entity {
                    **last_focus = None;
                    break;
                }
            }
        }
        if let Some(adapter) = adapters.get_primary_adapter() {
            let root_id = NodeId(NonZeroU128::new(WindowId::primary().as_u128()).unwrap());
            let children = remaining_nodes
                .iter()
                .map(|v| v.to_node_id())
                .collect::<Vec<NodeId>>();
            let window_update = (
                root_id,
                Arc::new(Node {
                    role: Role::Window,
                    children,
                    ..default()
                }),
            );
            let focus = (**last_focus).unwrap_or(root_id);
            adapter.update(TreeUpdate {
                nodes: vec![window_update],
                focus: Some(focus),
                ..default()
            });
        }
    }
}

pub struct AccessibilityPlugin;

impl Plugin for AccessibilityPlugin {
    fn build(&self, app: &mut App) {
        app.init_non_send_resource::<Adapters>()
            .init_resource::<Handlers>()
            .init_resource::<LastFocus>()
            .add_event::<ActionRequest>()
            .add_system_to_stage(CoreStage::PreUpdate, handle_focus)
            .add_system_to_stage(CoreStage::PreUpdate, window_closed)
            .add_system_to_stage(CoreStage::PreUpdate, poll_receivers)
            .add_system_to_stage(CoreStage::PreUpdate, update_accessibility_nodes)
            .add_system_to_stage(CoreStage::PostUpdate, remove_accessibility_nodes);
    }
}
