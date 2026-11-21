import 'package:flutter/widgets.dart';

/// Scroll controller of the chat feed (`.cv-feed`).
///
/// A browser keeps `scrollTop` when the page grows and the web chat then
/// sets it to the end if the reader was there. Here the same happens inside
/// layout, so no frame ever shows the feed away from where it must be:
///
/// * while [shouldPin] says so, the end of the feed stays at the bottom of
///   the view whenever the content or the view changes size (a new message,
///   a bubble that grows, the keyboard);
/// * [keepDistanceFromBottomOnce] keeps what the reader sees in place while
///   older messages are added above («Ver N mensajes anteriores»).
class PinnedScrollController extends ScrollController {
  PinnedScrollController({required this.shouldPin});

  final bool Function() shouldPin;
  double? _keepFromBottom;

  /// The next change of size keeps the current distance to the end.
  void keepDistanceFromBottomOnce() {
    if (!hasClients) return;
    final ScrollPosition p = position;
    if (p.hasContentDimensions) _keepFromBottom = p.maxScrollExtent - p.pixels;
  }

  double? _target(double min, double max) {
    final double? keep = _keepFromBottom;
    if (keep != null) {
      _keepFromBottom = null;
      return (max - keep).clamp(min, max);
    }
    return shouldPin() ? max : null;
  }

  @override
  ScrollPosition createScrollPosition(ScrollPhysics physics, ScrollContext context, ScrollPosition? oldPosition) =>
      _PinnedPosition(this, physics: physics, context: context, oldPosition: oldPosition, debugLabel: debugLabel);
}

class _PinnedPosition extends ScrollPositionWithSingleContext {
  _PinnedPosition(
    this._owner, {
    required super.physics,
    required super.context,
    super.oldPosition,
    super.debugLabel,
  });

  final PinnedScrollController _owner;

  @override
  bool applyContentDimensions(double minScrollExtent, double maxScrollExtent) {
    if (hasPixels) {
      final double? target = _owner._target(minScrollExtent, maxScrollExtent);
      if (target != null && (target - pixels).abs() > 0.01) correctPixels(target);
    }
    return super.applyContentDimensions(minScrollExtent, maxScrollExtent);
  }
}
