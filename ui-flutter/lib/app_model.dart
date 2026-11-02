import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'models.dart';
import 'rust_api.dart';

class LogEntry {
  LogEntry(this.at, this.message, {this.isError = false});

  final DateTime at;
  final String message;
  final bool isError;
}

/// Single-node mesh model: this device receives and mixes every peer while
/// sending its own capture. There is no center/sender split.
class AppModel extends ChangeNotifier {
  MeshState? mesh;
  bool busyMesh = false;
  String? lastError;
  final List<LogEntry> logs = [];

  BigInt? _meshId;
  Timer? _timer;

  /// Last join settings, so the new hardware-style panel can restore them.
  String nodeName = defaultNodeName();
  String lastSource = 'tone';
  bool lastOutput = true;
  double lastFreq = 440;
  double lastTargetMs = 100;
  double lastMasterGainDb = 0;

  double get targetMs => mesh?.center.targetMs ?? lastTargetMs;
  double get masterGainDb => mesh?.center.masterGainDb ?? lastMasterGainDb;

  String get sourceLabel => switch (lastSource) {
        'tone' => '测试音',
        'loopback' => '系统内录',
        'microphone' => '麦克风',
        _ => lastSource,
      };
  bool _disposed = false;
  bool _dirty = false;
  bool _meshPollError = false;
  bool _leaveRequested = false;
  bool _faultReported = false;
  final bool _uiDebug = Platform.environment['SIREN_UI_DEBUG'] == '1';
  int _pollCount = 0;

  AppModel() {
    _log('SirenStudio 已就绪');
    _timer = Timer.periodic(const Duration(milliseconds: 80), (_) => _poll());
    if (Platform.environment['SIREN_UI_AUTOSTART'] == '1') {
      final source = Platform.environment['SIREN_UI_MESH_SOURCE'] ?? 'tone';
      unawaited(
        Future<void>.delayed(
          const Duration(milliseconds: 800),
          () => startMesh(
            name: Platform.environment['SIREN_UI_MESH_NAME'] ??
                defaultNodeName(),
            source: source,
            output: Platform.environment['SIREN_UI_MESH_OUTPUT'] != '0',
          ),
        ),
      );
    }
  }

  bool get meshRunning => mesh?.running == true;

  Future<void> startMesh({
    required String name,
    required String source,
    required bool output,
    double freq = 440,
  }) async {
    if (busyMesh) return;
    busyMesh = true;
    nodeName = name;
    lastSource = source;
    lastOutput = output;
    lastFreq = freq;
    lastError = null;
    _meshPollError = false;
    notifyListeners();
    try {
      uiEvent('startMesh: begin');
      // A node that died on its own stays registered until we re-join: its
      // port is already free but the registry slot would reject the new start.
      final stale = _meshId;
      if (stale != null) {
        try {
          rust.meshStop(id: stale);
          rust.meshRelease(id: stale);
        } catch (_) {}
        _meshId = null;
      }
      _leaveRequested = false;
      _faultReported = false;
      final config = {
        'bind': '0.0.0.0',
        'port': int.tryParse(
              Platform.environment['SIREN_UI_MESH_PORT'] ?? '',
            ) ??
            47900,
        'name': name,
        'advertise': true,
        'output': output,
        'target_ms': double.tryParse(
              Platform.environment['SIREN_UI_MESH_TARGET_MS'] ?? '',
            ) ??
            lastTargetMs,
        'max_streams': 32,
        'source': source,
        'codec': 'raw',
        'frame_ms': 5,
        'freq': freq,
        'amplitude': 0.25,
        'discovery_secs': 5,
        'static_peers': <String>[],
        // Dual-stack is fully software-managed: Wi-Fi is always on and the
        // Bluetooth overlay joins automatically. No transport or channel
        // options are exposed.
      };
      final beginError = rust.beginStartMesh(configJson: jsonEncode(config));
      if (beginError.isNotEmpty) {
        throw Exception(beginError);
      }
      BigInt? id;
      final deadline = DateTime.now().add(const Duration(seconds: 20));
      while (DateTime.now().isBefore(deadline)) {
        final status = _status(rust.pollStartMesh());
        if (status.$1 == 'done') {
          id = BigInt.from(status.$2['id'] as int);
          break;
        }
        if (status.$1 == 'error') {
          throw Exception(status.$2['message'] ?? '启动失败');
        }
        await Future<void>.delayed(const Duration(milliseconds: 120));
      }
      if (id == null) {
        throw Exception('组网启动超时（20 秒）');
      }
      _meshId = id;
      final raw = rust.meshSnapshotJson(id: id);
      mesh = MeshState.fromJson(
        Map<String, dynamic>.from(jsonDecode(raw) as Map),
      );
      _log('已加入组网 · ${mesh!.bind} · ${output ? '本地播放' : '静默接收'}');
      uiEvent('startMesh: id=$id');
    } catch (error) {
      final message = _friendlyError(error);
      lastError = '加入组网失败：$message';
      _log(lastError!, isError: true);
      final id = _meshId;
      if (id != null) {
        try {
          rust.meshRelease(id: id);
        } catch (_) {}
        _meshId = null;
      }
    } finally {
      busyMesh = false;
      notifyListeners();
    }
  }

  void stopMesh() {
    final id = _meshId;
    if (id == null) return;
    _leaveRequested = true;
    if (rust.meshStop(id: id)) {
      _log('正在离开组网…');
      notifyListeners();
    } else {
      _log('离开组网失败：节点已不存在', isError: true);
      _finishLeave(id);
    }
  }

  void _finishLeave(BigInt id) {
    try {
      rust.meshRelease(id: id);
    } catch (_) {}
    _meshId = null;
    _leaveRequested = false;
    _faultReported = false;
  }

  void setStreamGain(int streamId, double gainDb) {
    final id = _meshId;
    if (id == null) return;
    rust.meshSetGainDb(id: id, stream: BigInt.from(streamId), gainDb: gainDb);
  }

  void setStreamMuted(int streamId, bool muted) {
    final id = _meshId;
    if (id == null) return;
    rust.meshSetMuted(id: id, stream: BigInt.from(streamId), muted: muted);
  }

  void setTargetMs(double ms) {
    lastTargetMs = ms;
    final id = _meshId;
    if (id == null) return;
    if (!rust.meshSetTargetMs(id: id, ms: ms)) {
      _log('设置缓冲目标失败', isError: true);
      notifyListeners();
    }
  }

  void setMasterGainDb(double db) {
    lastMasterGainDb = db;
    final id = _meshId;
    if (id == null) return;
    if (!rust.meshSetMasterGainDb(id: id, db: db)) {
      _log('设置主增益失败', isError: true);
      notifyListeners();
    }
  }

  void clearError() {
    lastError = null;
    notifyListeners();
  }

  Future<void> shutdown() async {
    _timer?.cancel();
    _timer = null;
    final meshId = _meshId;
    _meshId = null;
    if (meshId != null) {
      try {
        rust.meshStop(id: meshId);
      } catch (_) {}
    }
    await Future<void>.delayed(const Duration(milliseconds: 300));
    if (meshId != null) {
      try {
        rust.meshRelease(id: meshId);
      } catch (_) {}
    }
  }

  void _poll() {
    if (_disposed) return;
    var changed = _dirty;
    _dirty = false;

    final meshId = _meshId;
    if (meshId != null) {
      try {
        if (_pollCount < 4) uiEvent('poll #$_pollCount: snapshot begin');
        final decoded = jsonDecode(rust.meshSnapshotJson(id: meshId));
        if (decoded is Map) {
          final map = Map<String, dynamic>.from(decoded);
          if (map['error'] != null) {
            _finishLeave(meshId);
            _log('组网节点已释放');
            changed = true;
          } else {
            final next = MeshState.fromJson(map);
            mesh = next;
            changed = true;
            if (!next.running && _leaveRequested) {
              // The user asked to leave: complete the teardown now that the
              // sender loop is done.
              _finishLeave(meshId);
              _log('已离开组网');
            } else if (!next.running && !_faultReported) {
              // The node stopped without a leave request. Releasing it here
              // would turn a diagnosable fault into a vanished node, so keep
              // it registered and let the user re-join explicitly.
              _faultReported = true;
              lastError = '组网节点已停止，请重新点击「加入组网」';
              _log(lastError!, isError: true);
            }
          }
        }
        _meshPollError = false;
      } catch (error) {
        if (!_meshPollError) {
          _meshPollError = true;
          _log('读取组网状态失败：${_friendlyError(error)}', isError: true);
          changed = true;
        }
      }
    }

    _pollCount++;
    if (changed) notifyListeners();
  }

  /// Returns `(state, payload)` from a `begin/poll` status JSON string.
  (String, Map<String, dynamic>) _status(String json) {
    try {
      final decoded = jsonDecode(json);
      if (decoded is Map) {
        final map = Map<String, dynamic>.from(decoded);
        return ((map['state'] as String?) ?? 'idle', map);
      }
    } catch (_) {}
    return ('idle', const {});
  }

  void uiEvent(String message) {
    if (!_uiDebug) return;
    try {
      File('ui_debug.log').writeAsStringSync(
        '${DateTime.now().toIso8601String()} $message\n',
        mode: FileMode.append,
        flush: true,
      );
    } catch (_) {}
  }

  void _log(String message, {bool isError = false}) {
    logs.add(LogEntry(DateTime.now(), message, isError: isError));
    if (logs.length > 80) logs.removeRange(0, logs.length - 80);
    _dirty = true;
  }

  String _friendlyError(Object error) {
    var text = error.toString().trim();
    text = text.replaceFirst(
      RegExp(r'^(AnyhowException|Exception|PanicException)\s*[:(]\s*'),
      '',
    );
    while (text.endsWith(')')) {
      text = text.substring(0, text.length - 1);
    }
    return text.trim();
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    _timer = null;
    final meshId = _meshId;
    _meshId = null;
    if (meshId != null) {
      try {
        rust.meshStop(id: meshId);
        rust.meshRelease(id: meshId);
      } catch (_) {}
    }
    super.dispose();
  }
}

String defaultNodeName() {
  if (Platform.isWindows) {
    return Platform.environment['COMPUTERNAME'] ?? 'Siren Node';
  }
  return Platform.environment['HOSTNAME'] ?? 'Siren Node';
}
