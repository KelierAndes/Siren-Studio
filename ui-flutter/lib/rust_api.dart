import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

typedef _CNoArgs = Pointer<Utf8> Function();
typedef _DNoArgs = Pointer<Utf8> Function();
typedef _CStr = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _CFree = Void Function(Pointer<Utf8>);
typedef _DStr = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _DFree = void Function(Pointer<Utf8>);
typedef _CU64Str = Pointer<Utf8> Function(Uint64);
typedef _DU64Str = Pointer<Utf8> Function(int);
typedef _CU64U64Float = Int32 Function(Uint64, Uint64, Float);
typedef _DU64U64Float = int Function(int, int, double);
typedef _CU64Float = Int32 Function(Uint64, Float);
typedef _DU64Float = int Function(int, double);
typedef _CU64U64I32 = Int32 Function(Uint64, Uint64, Int32);
typedef _DU64U64I32 = int Function(int, int, int);
typedef _CU64 = Int32 Function(Uint64);
typedef _DU64 = int Function(int);
typedef _CU32 = Void Function(Uint32);
typedef _DU32 = void Function(int);
typedef _CVoid = Void Function();
typedef _DVoid = void Function();

/// In-process binding to the Rust mesh node library.
late final RustApi rust;

class RustApi {
  RustApi._(this._lib);

  final DynamicLibrary _lib;

  late final _DNoArgs _version =
      _lib.lookupFunction<_CNoArgs, _DNoArgs>('siren_version');
  late final _DVoid _initLogging =
      _lib.lookupFunction<_CVoid, _DVoid>('siren_init_logging');
  late final _DFree _free =
      _lib.lookupFunction<_CFree, _DFree>('siren_string_free');
  late final _DU32 _trace =
      _lib.lookupFunction<_CU32, _DU32>('siren_trace');
  late final _DStr _beginStartMesh =
      _lib.lookupFunction<_CStr, _DStr>('siren_begin_start_mesh');
  late final _DNoArgs _pollStartMesh =
      _lib.lookupFunction<_CNoArgs, _DNoArgs>('siren_poll_start_mesh');
  late final _DU64Str _meshSnapshot =
      _lib.lookupFunction<_CU64Str, _DU64Str>('siren_mesh_snapshot');
  late final _DU64U64Float _meshSetGain =
      _lib.lookupFunction<_CU64U64Float, _DU64U64Float>('siren_mesh_set_gain');
  late final _DU64U64I32 _meshSetMuted =
      _lib.lookupFunction<_CU64U64I32, _DU64U64I32>('siren_mesh_set_muted');
  late final _DU64Float _meshSetTargetMs =
      _lib.lookupFunction<_CU64Float, _DU64Float>('siren_mesh_set_target_ms');
  late final _DU64Float _meshSetMasterGainDb = _lib
      .lookupFunction<_CU64Float, _DU64Float>('siren_mesh_set_master_gain_db');
  late final _DU64 _meshStop =
      _lib.lookupFunction<_CU64, _DU64>('siren_mesh_stop');
  late final _DU64 _meshRelease =
      _lib.lookupFunction<_CU64, _DU64>('siren_mesh_release');
  late final _DU64 _meshIsRunning =
      _lib.lookupFunction<_CU64, _DU64>('siren_mesh_is_running');

  static RustApi load() {
    final library = Platform.isAndroid
        ? DynamicLibrary.open('librust_lib_siren_ui.so')
        : DynamicLibrary.open('rust_lib_siren_ui.dll');
    return RustApi._(library);
  }

  String _take(Pointer<Utf8> pointer) {
    if (pointer.address == 0) return '';
    final value = pointer.toDartString();
    _free(pointer);
    return value;
  }

  void initLogging() => _initLogging();

  void trace(int id) => _trace(id);

  String version() {
    _trace(21);
    return _take(_version());
  }

  String beginStartMesh({required String configJson}) {
    _trace(30);
    final pointer = configJson.toNativeUtf8();
    try {
      return _take(_beginStartMesh(pointer));
    } finally {
      calloc.free(pointer);
    }
  }

  String pollStartMesh() {
    _trace(31);
    return _take(_pollStartMesh());
  }

  String meshSnapshotJson({required BigInt id}) {
    _trace(32);
    return _take(_meshSnapshot(id.toInt()));
  }

  bool meshSetGainDb({
    required BigInt id,
    required BigInt stream,
    required double gainDb,
  }) =>
      _meshSetGain(id.toInt(), stream.toInt(), gainDb) != 0;

  bool meshSetMuted({
    required BigInt id,
    required BigInt stream,
    required bool muted,
  }) =>
      _meshSetMuted(id.toInt(), stream.toInt(), muted ? 1 : 0) != 0;

  bool meshSetTargetMs({required BigInt id, required double ms}) =>
      _meshSetTargetMs(id.toInt(), ms) != 0;

  bool meshSetMasterGainDb({required BigInt id, required double db}) =>
      _meshSetMasterGainDb(id.toInt(), db) != 0;

  bool meshStop({required BigInt id}) => _meshStop(id.toInt()) != 0;

  bool meshRelease({required BigInt id}) => _meshRelease(id.toInt()) != 0;

  bool meshIsRunning({required BigInt id}) => _meshIsRunning(id.toInt()) != 0;
}
