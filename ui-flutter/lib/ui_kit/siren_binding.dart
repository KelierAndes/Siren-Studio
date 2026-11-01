import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../app_model.dart';
import '../models.dart';
import 'model.dart';
import 'theme.dart';

/// Bridges the mesh engine to the hardware-style panel.
///
/// The visual model is the original BASSMAKER device model; here its four
/// modules, faders and bottom strip are wired to real Siren functions:
///   module 0/1 = remote streams (gain / target / mute)
///   module 2   = local node (master gain / target / tone frequency)
///   module 3   = mesh link (target buffer, Bluetooth state)
class SirenBinding {
  SirenBinding(this.app) : visual = ShaperModel() {
    _wire();
    app.addListener(_sync);
    visual.addListener(_onVisual);
    _lastPage = visual.streamPage;
    _lastTheme = visual.theme;
    _sync();
  }

  final AppModel app;
  final ShaperModel visual;

  VoidCallback? onRequestJoin;
  VoidCallback? onRequestSettings;

  bool _syncing = false;
  double _lastAudibleGain = 0;
  int _lastPage = 0;
  AppThemeMode _lastTheme = AppThemeMode.dark;
  final Map<int, (int, int)> _counters = {};
  final Map<int, DateTime> _lastIssue = {};

  void _wire() {
    visual.onPower = () {
      if (app.meshRunning) {
        app.stopMesh();
      } else {
        onRequestJoin?.call();
      }
    };
    visual.onSettings = () => onRequestSettings?.call();
    visual.onNameChanged = (value) => app.nodeName = value;
    visual.onSourceChanged = (value) => app.lastSource = value;
    visual.onToneFreqChanged = (value) => app.lastFreq = value;
    visual.onOutputChanged = (value) => app.lastOutput = value;
    visual.onTargetChanged = app.setTargetMs;
    visual.onMasterChanged = app.setMasterGainDb;
    visual.onJoinToggle = () {
      if (app.busyMesh) return;
      if (app.meshRunning) {
        app.stopMesh();
        return;
      }
      final name = app.nodeName.trim().isEmpty ? 'Siren Node' : app.nodeName.trim();
      unawaited(
        app.startMesh(
          name: name,
          source: app.lastSource,
          output: app.lastOutput,
          freq: app.lastFreq,
        ),
      );
    };
    visual.onNext = () => onRequestSettings?.call();
    visual.onPrev = visual.toggleTheme;
    visual.onHp = (value) {
      if (_syncing) return;
      app.setTargetMs(value);
    };
    visual.onLp = (value) {
      if (_syncing) return;
      app.setMasterGainDb(value);
    };
    visual.onBypass = () {
      if (_syncing) return;
      final gain = app.masterGainDb;
      if (gain <= -59) {
        app.setMasterGainDb(_lastAudibleGain);
      } else {
        _lastAudibleGain = gain;
        app.setMasterGainDb(-60);
      }
    };

    for (var i = 0; i < 2; i++) {
      final channel = visual.channels[i];
      channel.onAmount = (value) {
        if (_syncing) return;
        final stream = _streamAt(i);
        if (stream != null) app.setStreamGain(stream.id, value);
      };
      channel.onA = (value) {
        if (_syncing) return;
        app.setTargetMs(value);
      };
      channel.onB = (value) {
        if (_syncing) return;
        app.setMasterGainDb(value);
      };
      channel.onMute = (muted) {
        if (_syncing) return;
        final stream = _streamAt(i);
        if (stream != null) app.setStreamMuted(stream.id, muted);
      };
    }

    final node = visual.channels[2];
    node.onAmount = (value) {
      if (_syncing) return;
      app.setMasterGainDb(value);
    };
    node.onA = (value) {
      if (_syncing) return;
      app.setTargetMs(value);
    };
    node.onB = (value) {
      if (_syncing) return;
      app.lastFreq = value;
    };

    final link = visual.channels[3];
    link.onAmount = (value) {
      if (_syncing) return;
      app.setTargetMs(value);
    };
  }

  /// Stream shown in card [index] on the current page.
  StreamState? _streamAt(int index) {
    final streams = app.mesh?.center.streams ?? const <StreamState>[];
    final at = visual.streamPage * 2 + index;
    return at < streams.length ? streams[at] : null;
  }

  void _onVisual() {
    if (_syncing) return;
    if (_lastPage != visual.streamPage || _lastTheme != visual.theme) {
      _lastPage = visual.streamPage;
      _lastTheme = visual.theme;
      _sync();
    }
  }

  Color _curveColor(int index) {
    final p = visual.palette;
    final accents = p.isLight
        ? const [Color(0xFF1A5EAE), Color(0xFFA55A05)]
        : const [Color(0xFF5BA7FF), Color(0xFFFFA23E)];
    if (index < accents.length) return accents[index];
    return [p.cyan, p.violet, p.salmon, p.orange][(index - accents.length) % 4];
  }

  void _sync() {
    final mesh = app.mesh;
    final running = mesh?.running == true;
    _syncing = true;
    try {
      visual.setJoined(running);
      final streams = mesh?.center.streams ?? const <StreamState>[];
      final origins = <String>{
        for (final stream in streams)
          if (stream.streamId != 0xFFF0) stream.origin,
      };
      final discovered = (mesh?.peers.length ?? 0) + 1;
      final devices = discovered > origins.length + 1 ? discovered : origins.length + 1;
      if (running) {
        visual.setStatusText('组网中 · $devices 台');
      } else {
        visual.setStatusText('未组网');
      }

      final tx = mesh?.senderRmsDb ?? -48;
      final out = mesh?.center.master.rmsDb ?? -48;
      visual.setLevels(tx, out);

      final seen = <int>{};
      final health = <int, int>{};
      final now = DateTime.now();
      for (final stream in streams) {
        seen.add(stream.id);
        final prev = _counters[stream.id];
        if (prev != null &&
            (stream.lostFrames > prev.$1 || stream.underruns > prev.$2)) {
          _lastIssue[stream.id] = now;
        }
        _counters[stream.id] = (stream.lostFrames, stream.underruns);
        if (!stream.active) {
          health[stream.id] = 3;
          continue;
        }
        final issue = _lastIssue[stream.id];
        if (issue == null) {
          health[stream.id] = 0;
        } else {
          final age = now.difference(issue);
          health[stream.id] = age < const Duration(seconds: 1)
              ? 2
              : age < const Duration(seconds: 4)
                  ? 1
                  : 0;
        }
      }
      _counters.removeWhere((id, _) => !seen.contains(id));
      _lastIssue.removeWhere((id, _) => !seen.contains(id));

      final curves = <EqCurve>[];
      for (final stream in streams) {
        // 0xFFF0 is the hub relay mix, not a device: it would swamp the chart.
        if (stream.streamId == 0xFFF0) continue;
        curves.add(
          EqCurve(
            name: mesh!.streamName(stream),
            color: _curveColor(curves.length),
            bands: stream.spectrum,
            gainDb: stream.gainDb,
            active: stream.active && !stream.muted,
          ),
        );
      }
      visual.setEqCurves(curves);
      visual.setStreamPages((streams.length + 1) ~/ 2);
      visual.setStreamPage(visual.streamPage);
      _lastPage = visual.streamPage;

      final target = (mesh?.center.targetMs ?? app.lastTargetMs).clamp(10.0, 500.0);
      final master = (mesh?.center.masterGainDb ?? app.lastMasterGainDb).clamp(-60.0, 12.0);
      visual.setHp(target);
      visual.setLp(master);
      visual.syncBypass(master <= -59);

      for (var i = 0; i < 2; i++) {
        final channel = visual.channels[i];
        final stream = _streamAt(i);
        if (stream == null) {
          channel.setTitle('等待节点');
          channel.setReadout('缓冲', '目标', '--', 'ms');
          channel.setInfo('—', '—');
          channel.setFacts(rms: -60, path: '—', lost: 0, under: 0, health: 3);
          channel.syncAmount(-60);
          channel.syncMute(false);
          channel.syncOn(false);
        } else {
          channel.setTitle(mesh!.streamName(stream));
          channel.setReadout(
            '缓冲',
            '目标',
            '${stream.bufferMs.round()}/${stream.targetMs.round()}',
            'ms',
          );
          channel.setInfo(
            '${_pathLabel(stream.path)} · ${stream.codec.toUpperCase()}',
            '${stream.lostFrames}',
          );
          channel.setFacts(
            rms: stream.rmsDb,
            path: _pathLabel(stream.path),
            lost: stream.lostFrames,
            under: stream.underruns,
            health: health[stream.id] ?? 3,
          );
          channel.setFreq(stream.bufferMs.clamp(channel.freqRange.min, channel.freqRange.max));
          channel.syncAmount(stream.gainDb);
          channel.syncMute(stream.muted);
          channel.syncOn(stream.active);
        }
      }

      final node = visual.channels[2];
      node.setTitle('本机');
      node.setReadout('音源', '输出', app.sourceLabel, '');
      node.setInfo(app.lastOutput ? '本地播放' : '静默接收', '${app.lastFreq.round()} Hz');
      node.setFreq(app.lastFreq);
      node.syncAmount(master);
      node.setA(target);
      node.setB(app.lastFreq);
      node.syncOn(running);

      final link = visual.channels[3];
      final bt = mesh?.bt;
      final role = switch (bt?.role) {
        'hub' => '中心',
        'leaf' => '节点',
        _ => '等待',
      };
      final state = switch (bt?.state) {
        'listening' => '监听',
        'connected' => '已连',
        _ => '等待',
      };
      link.setTitle('组网');
      link.setReadout('蓝牙', '链路', '$role/$state', '');
      link.setInfo('$role · $state', bt == null ? '等待' : '${bt.links} 路');
      link.syncAmount(target);
      link.setFreq(target);
      link.setA(bt == null ? 0 : (bt.links * 50.0).clamp(0, 400));
      link.setB(bt?.channel.toDouble() ?? 16);
      link.syncOn(running);

      final names = <String>[];
      for (final peer in [...?mesh?.peers, ...?mesh?.topology]) {
        final name = peer.displayName;
        if (!names.contains(name)) names.add(name);
      }
      visual.setPeers(names);
      visual.setInlineSettings(
        name: app.nodeName,
        source: app.lastSource,
        tone: app.lastFreq,
        out: app.lastOutput,
        target: target,
        master: master,
        isBusy: app.busyMesh,
        detail: bt == null ? '' : '$role · $state · ${bt.links} 路',
        peers: origins.length,
        devices: running ? devices : 1,
      );
      _lastTheme = visual.theme;
    } finally {
      _syncing = false;
    }
  }

  AppModel get model => app;

  static String _pathLabel(String path) => switch (path) {
        'wifi' => 'Wi-Fi',
        'bt' => '蓝牙',
        'local' => '本地',
        _ => path.isEmpty ? '—' : path,
      };

  void dispose() {
    app.removeListener(_sync);
    visual.removeListener(_onVisual);
  }
}
