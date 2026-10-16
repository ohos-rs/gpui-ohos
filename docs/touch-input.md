# OHOS touch input and frame scheduling

By default, `src/ohos/window.rs` advances rendering and momentum through
XComponent frame callbacks. Each window registers on demand and unregisters after
12 idle callbacks. Registration changes run on the UI thread outside native frame
callbacks. A failed registration schedules one fallback draw after at least
16 ms; new demand retries registration. Failed unregistration leaves continuous
callbacks enabled until surface destruction.

With the optional `native-vsync` feature, the embedding application can select
NativeVSync per window and switch back to XComponent at runtime using
[`configure_native_refresh`](../README.md#optional-nativevsync-delivery).
Both sources use the same frame demand and momentum handling. Native callbacks
are dispatched to the UI thread, with the next frame armed before drawing.
Source switches and receiver destruction are deferred outside frame callbacks;
generation tokens reject native callbacks queued before a switch or teardown.
Native API failures fall back to XComponent. Hidden or idle windows release the
native receiver, and later demand recreates it.

Touch scrolling is adapted by `src/ohos/touch_scroll.rs`, using only GPUI's
public `PlatformInput` interface. GPUI itself is the unmodified git dependency
at revision `f45c7c22d0c04fb12527e71bbbee1dc6bdbdee0a`; no local GPUI source
patch or extra `TouchEvent` field is required.

The framework enables `TouchInputDelivery::Both`. XComponent's system ArkUI
Pan supplies recognition, physical-pixel deltas and release velocity in px/s.
The window converts displacement and velocity to GPUI logical pixels once.
There is no raw-touch pan threshold, velocity history or least-squares fit.

The adapter offers raw `Started` contacts to GPUI so controls can claim a
direct drag. Claimed drags keep their complete raw stream. Unclaimed contacts
are immediately cancelled in GPUI. Compatibility taps use
the ordinary mouse compatibility events, long presses use public phased
`LongPressEvent`. Only native Pan events emit scrolling `ScrollWheelEvent`.
Pan Start applies the system-provided displacement immediately, so the first
accepted packet invalidates the view without waiting for Update. Later packets
are incremental and never replay that initial displacement. A caught fling stops
on raw contact Down;
the next native pan chooses its own axis and cannot become a tap. Raw Up and
Pan End may arrive in either order, and neither launches a second fling.
The existing native recognizer uses 8 physical pixels, matching the SDK's
native gesture units. The OHOS ArkUI friction curve is retained.
After compatibility taps, an idle raw cancellation restores GPUI's touch
input modality without emitting another gesture.

Native Pan does not emit post-release displacement. Each window therefore owns
at most one momentum curve, seeded from native Pan End velocity and advanced
once per delivered frame using `Instant::now()`. Its origin is release dispatch
time; a delayed frame integrates elapsed motion without replaying intermediate
frames, and a frame time before release cannot advance the curve. Swipe does not
launch another curve. No GPUI momentum frame callbacks are scheduled. Pending
long presses and frame-registration failure recovery use cancellable, one-shot
timers.

Hiding or destroying a surface unregisters its frame callback and cancels any
pending fallback draw. Hidden or destroyed surfaces do not present frames;
backgrounding clears pending frame demand, active contacts, long-press timers
and momentum. A
visible surface requests a fresh frame and registers its callback on resume.
This prevents a queued animation frame from presenting to a surface the
system has just hidden or destroyed.
