/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! This module contains traits in script used generically in the rest of Servo.
//! The traits are here instead of in script so that these modules won't have
//! to depend on script.

#![feature(custom_derive, plugin)]
#![plugin(plugins, serde_macros)]
#![deny(missing_docs)]

extern crate app_units;
extern crate canvas_traits;
extern crate devtools_traits;
extern crate euclid;
extern crate gfx_traits;
extern crate ipc_channel;
extern crate libc;
extern crate msg;
extern crate net_traits;
extern crate offscreen_gl_context;
extern crate profile_traits;
extern crate serde;
extern crate style_traits;
extern crate time;
extern crate url;
extern crate util;

mod script_msg;

use app_units::Au;
use devtools_traits::ScriptToDevtoolsControlMsg;
use euclid::Size2D;
use euclid::length::Length;
use euclid::point::Point2D;
use euclid::rect::Rect;
use gfx_traits::LayerId;
use ipc_channel::ipc::{IpcReceiver, IpcSender};
use libc::c_void;
use msg::compositor_msg::Epoch;
use msg::constellation_msg::{ConstellationChan, Failure, PipelineId, WindowSizeData};
use msg::constellation_msg::{Key, KeyModifiers, KeyState, LoadData, SubpageId};
use msg::constellation_msg::{MouseButton, MouseEventType};
use msg::constellation_msg::{MozBrowserEvent, PipelineNamespaceId};
use msg::webdriver_msg::WebDriverScriptCommand;
use net_traits::ResourceThread;
use net_traits::image_cache_thread::ImageCacheThread;
use net_traits::storage_thread::StorageThread;
use profile_traits::mem;
use std::any::Any;
use util::ipc::OptionalOpaqueIpcSender;
use util::mem::HeapSizeOf;

pub use script_msg::{LayoutMsg, ScriptMsg};

/// The address of a node. Layout sends these back. They must be validated via
/// `from_untrusted_node_address` before they can be used, because we do not trust layout.
#[derive(Copy, Clone, Debug)]
pub struct UntrustedNodeAddress(pub *const c_void);
unsafe impl Send for UntrustedNodeAddress {}

/// Messages sent to the layout thread from the constellation and/or compositor.
#[derive(Deserialize, Serialize)]
pub enum LayoutControlMsg {
    /// Requests that this layout thread exit.
    ExitNow,
    /// Requests the current epoch (layout counter) from this layout.
    GetCurrentEpoch(IpcSender<Epoch>),
    /// Asks layout to run another step in its animation.
    TickAnimations,
    /// Informs layout as to which regions of the page are visible.
    SetVisibleRects(Vec<(LayerId, Rect<Au>)>),
    /// Requests the current load state of Web fonts. `true` is returned if fonts are still loading
    /// and `false` is returned if all fonts have loaded.
    GetWebFontLoadState(IpcSender<bool>),
}

/// The initial data associated with a newly-created framed pipeline.
#[derive(Deserialize, Serialize)]
pub struct NewLayoutInfo {
    /// Id of the parent of this new pipeline.
    pub containing_pipeline_id: PipelineId,
    /// Id of the newly-created pipeline.
    pub new_pipeline_id: PipelineId,
    /// Id of the new frame associated with this pipeline.
    pub subpage_id: SubpageId,
    /// Network request data which will be initiated by the script thread.
    pub load_data: LoadData,
    /// The paint channel, cast to `OptionalOpaqueIpcSender`. This is really an
    /// `Sender<LayoutToPaintMsg>`.
    pub paint_chan: OptionalOpaqueIpcSender,
    /// Information on what to do on thread failure.
    pub failure: Failure,
    /// A port on which layout can receive messages from the pipeline.
    pub pipeline_port: IpcReceiver<LayoutControlMsg>,
    /// A shutdown channel so that layout can notify others when it's done.
    pub layout_shutdown_chan: IpcSender<()>,
    /// A shutdown channel so that layout can tell the content process to shut down when it's done.
    pub content_process_shutdown_chan: IpcSender<()>,
}

/// Messages sent from the constellation or layout to the script thread.
#[derive(Deserialize, Serialize)]
pub enum ConstellationControlMsg {
    /// Gives a channel and ID to a layout thread, as well as the ID of that layout's parent
    AttachLayout(NewLayoutInfo),
    /// Window resized.  Sends a DOM event eventually, but first we combine events.
    Resize(PipelineId, WindowSizeData),
    /// Notifies script that window has been resized but to not take immediate action.
    ResizeInactive(PipelineId, WindowSizeData),
    /// Notifies the script that a pipeline should be closed.
    ExitPipeline(PipelineId),
    /// Sends a DOM event.
    SendEvent(PipelineId, CompositorEvent),
    /// Notifies script of the viewport.
    Viewport(PipelineId, Rect<f32>),
    /// Requests that the script thread immediately send the constellation the title of a pipeline.
    GetTitle(PipelineId),
    /// Notifies script thread to suspend all its timers
    Freeze(PipelineId),
    /// Notifies script thread to resume all its timers
    Thaw(PipelineId),
    /// Notifies script thread that a url should be loaded in this iframe.
    Navigate(PipelineId, SubpageId, LoadData),
    /// Requests the script thread forward a mozbrowser event to an iframe it owns
    MozBrowserEvent(PipelineId, SubpageId, MozBrowserEvent),
    /// Updates the current subpage id of a given iframe
    UpdateSubpageId(PipelineId, SubpageId, SubpageId),
    /// Set an iframe to be focused. Used when an element in an iframe gains focus.
    FocusIFrame(PipelineId, SubpageId),
    /// Passes a webdriver command to the script thread for execution
    WebDriverScriptCommand(PipelineId, WebDriverScriptCommand),
    /// Notifies script thread that all animations are done
    TickAllAnimations(PipelineId),
    /// Notifies the script thread that a new Web font has been loaded, and thus the page should be
    /// reflowed.
    WebFontLoaded(PipelineId),
    /// Cause a `load` event to be dispatched at the appropriate frame element.
    DispatchFrameLoadEvent {
        /// The pipeline that has been marked as loaded.
        target: PipelineId,
        /// The pipeline that contains a frame loading the target pipeline.
        parent: PipelineId
    },
    /// Report an error from a CSS parser for the given pipeline
    ReportCSSError(PipelineId, String, usize, usize, String),
}

/// The type of input represented by a multi-touch event.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum TouchEventType {
    /// A new touch point came in contact with the screen.
    Down,
    /// An existing touch point changed location.
    Move,
    /// A touch point was removed from the screen.
    Up,
    /// The system stopped tracking a touch point.
    Cancel,
}

/// An opaque identifier for a touch point.
///
/// http://w3c.github.io/touch-events/#widl-Touch-identifier
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TouchId(pub i32);

/// Events from the compositor that the script thread needs to know about
#[derive(Deserialize, Serialize)]
pub enum CompositorEvent {
    /// The window was resized.
    ResizeEvent(WindowSizeData),
    /// A mouse button state changed.
    MouseButtonEvent(MouseEventType, MouseButton, Point2D<f32>),
    /// The mouse was moved over a point (or was moved out of the recognizable region).
    MouseMoveEvent(Option<Point2D<f32>>),
    /// A touch event was generated with a touch ID and location.
    TouchEvent(TouchEventType, TouchId, Point2D<f32>),
    /// A key was pressed.
    KeyEvent(Key, KeyState, KeyModifiers),
}

/// An opaque wrapper around script<->layout channels to avoid leaking message types into
/// crates that don't need to know about them.
pub struct OpaqueScriptLayoutChannel(pub (Box<Any + Send>, Box<Any + Send>));

/// Requests a TimerEvent-Message be sent after the given duration.
#[derive(Deserialize, Serialize)]
pub struct TimerEventRequest(pub IpcSender<TimerEvent>,
                             pub TimerSource,
                             pub TimerEventId,
                             pub MsDuration);

/// Notifies the script thread to fire due timers.
/// TimerSource must be FromWindow when dispatched to ScriptThread and
/// must be FromWorker when dispatched to a DedicatedGlobalWorkerScope
#[derive(Deserialize, Serialize)]
pub struct TimerEvent(pub TimerSource, pub TimerEventId);

/// Describes the thread that requested the TimerEvent.
#[derive(Copy, Clone, HeapSizeOf, Deserialize, Serialize)]
pub enum TimerSource {
    /// The event was requested from a window (ScriptThread).
    FromWindow(PipelineId),
    /// The event was requested from a worker (DedicatedGlobalWorkerScope).
    FromWorker
}

/// The id to be used for a TimerEvent is defined by the corresponding TimerEventRequest.
#[derive(PartialEq, Eq, Copy, Clone, Debug, HeapSizeOf, Deserialize, Serialize)]
pub struct TimerEventId(pub u32);

/// Unit of measurement.
#[derive(Clone, Copy, HeapSizeOf)]
pub enum Milliseconds {}
/// Unit of measurement.
#[derive(Clone, Copy, HeapSizeOf)]
pub enum Nanoseconds {}

/// Amount of milliseconds.
pub type MsDuration = Length<Milliseconds, u64>;
/// Amount of nanoseconds.
pub type NsDuration = Length<Nanoseconds, u64>;

/// Returns the duration since an unspecified epoch measured in ms.
pub fn precise_time_ms() -> MsDuration {
    Length::new(time::precise_time_ns() / (1000 * 1000))
}
/// Returns the duration since an unspecified epoch measured in ns.
pub fn precise_time_ns() -> NsDuration {
    Length::new(time::precise_time_ns())
}

/// Data needed to construct a script thread.
///
/// NB: *DO NOT* add any Senders or Receivers here! pcwalton will have to rewrite your code if you
/// do! Use IPC senders and receivers instead.
pub struct InitialScriptState {
    /// The ID of the pipeline with which this script thread is associated.
    pub id: PipelineId,
    /// The subpage ID of this pipeline to create in its pipeline parent.
    /// If `None`, this is the root.
    pub parent_info: Option<(PipelineId, SubpageId)>,
    /// The compositor.
    pub compositor: IpcSender<ScriptToCompositorMsg>,
    /// A channel with which messages can be sent to us (the script thread).
    pub control_chan: IpcSender<ConstellationControlMsg>,
    /// A port on which messages sent by the constellation to script can be received.
    pub control_port: IpcReceiver<ConstellationControlMsg>,
    /// A channel on which messages can be sent to the constellation from script.
    pub constellation_chan: ConstellationChan<ScriptMsg>,
    /// A channel for the layout thread to send messages to the constellation.
    pub layout_to_constellation_chan: ConstellationChan<LayoutMsg>,
    /// A channel to schedule timer events.
    pub scheduler_chan: IpcSender<TimerEventRequest>,
    /// Information that script sends out when it panics.
    pub failure_info: Failure,
    /// A channel to the resource manager thread.
    pub resource_thread: ResourceThread,
    /// A channel to the storage thread.
    pub storage_thread: StorageThread,
    /// A channel to the image cache thread.
    pub image_cache_thread: ImageCacheThread,
    /// A channel to the time profiler thread.
    pub time_profiler_chan: profile_traits::time::ProfilerChan,
    /// A channel to the memory profiler thread.
    pub mem_profiler_chan: mem::ProfilerChan,
    /// A channel to the developer tools, if applicable.
    pub devtools_chan: Option<IpcSender<ScriptToDevtoolsControlMsg>>,
    /// Information about the initial window size.
    pub window_size: Option<WindowSizeData>,
    /// The ID of the pipeline namespace for this script thread.
    pub pipeline_namespace_id: PipelineNamespaceId,
    /// A ping will be sent on this channel once the script thread shuts down.
    pub content_process_shutdown_chan: IpcSender<()>,
}

/// Encapsulates external communication with the script thread.
#[derive(Clone, Deserialize, Serialize)]
pub struct ScriptControlChan(pub IpcSender<ConstellationControlMsg>);

/// This trait allows creating a `ScriptThread` without depending on the `script`
/// crate.
pub trait ScriptThreadFactory {
    /// Create a `ScriptThread`.
    fn create(_phantom: Option<&mut Self>,
              state: InitialScriptState,
              layout_chan: &OpaqueScriptLayoutChannel,
              load_data: LoadData);
    /// Create a script -> layout channel (`Sender`, `Receiver` pair).
    fn create_layout_channel(_phantom: Option<&mut Self>) -> OpaqueScriptLayoutChannel;
    /// Clone the `Sender` in `pair`.
    fn clone_layout_channel(_phantom: Option<&mut Self>, pair: &OpaqueScriptLayoutChannel)
                            -> Box<Any + Send>;
}

/// Messages sent from the script thread to the compositor
#[derive(Deserialize, Serialize)]
pub enum ScriptToCompositorMsg {
    /// Scroll a page in a window
    ScrollFragmentPoint(PipelineId, LayerId, Point2D<f32>, bool),
    /// Set title of current page
    /// https://html.spec.whatwg.org/multipage/#document.title
    SetTitle(PipelineId, Option<String>),
    /// Send a key event
    SendKeyEvent(Key, KeyState, KeyModifiers),
    /// Get Window Informations size and position
    GetClientWindow(IpcSender<(Size2D<u32>, Point2D<i32>)>),
    /// Move the window to a point
    MoveTo(Point2D<i32>),
    /// Resize the window to size
    ResizeTo(Size2D<u32>),
    /// Script has handled a touch event, and either prevented or allowed default actions.
    TouchEventProcessed(EventResult),
    /// Requests that the compositor shut down.
    Exit,
}

/// Whether a DOM event was prevented by web content
#[derive(Deserialize, Serialize)]
pub enum EventResult {
    /// Allowed by web content
    DefaultAllowed,
    /// Prevented by web content
    DefaultPrevented,
}
