/// A tiny service locator. No reflection, no code generation.
///
/// Services are registered once in `bootstrap.dart` (and by the voice, car and
/// notifications features through their own `register…` functions) and read
/// with [Locator.get] / [Locator.maybe]. Widgets never construct services.
class Locator {
  final Map<Type, Object> _services = <Type, Object>{};

  /// Registers (or replaces) the instance for type [T].
  void register<T extends Object>(T instance) => _services[T] = instance;

  /// Removes the instance for type [T], if any.
  void unregister<T extends Object>() => _services.remove(T);

  /// Returns the instance for [T]; throws if it was never registered.
  T get<T extends Object>() {
    final Object? service = _services[T];
    if (service == null) {
      throw StateError('Service $T is not registered');
    }
    return service as T;
  }

  /// Returns the instance for [T], or null when nothing is registered.
  T? maybe<T extends Object>() => _services[T] as T?;

  /// Only for tests.
  void reset() => _services.clear();
}

/// The app-wide locator.
final Locator sl = Locator();
