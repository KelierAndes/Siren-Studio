double _d(dynamic value) => value is num ? value.toDouble() : 0.0;
double? _dn(dynamic value) => value is num ? value.toDouble() : null;
int _i(dynamic value) => value is num ? value.toInt() : 0;
bool _b(dynamic value) => value == true;
String _s(dynamic value, [String fallback = '']) => value is String ? value : fallback;
String? _sn(dynamic value) => value is String && value.isNotEmpty ? value : null;

Map<String, dynamic> _map(dynamic value) =>
    value is Map ? Map<String, dynamic>.from(value) : const {};

List<T> _list<T>(dynamic value, T Function(Map<String, dynamic>) parse) {
  if (value is! List) return const [];
  return value
      .whereType<Map>()
      .map((item) => parse(Map<String, dynamic>.from(item)))
      .toList(growable: false);
}

class MasterState {
  const MasterState({
    required this.rmsDb,
    required this.peakDb,
    required this.peakHoldDb,
    required this.framesRendered,
    required this.clippedSamples,
    required this.activeStreams,
  });

  final double rmsDb;
  final double peakDb;
  final double peakHoldDb;
  final int framesRendered;
  final int clippedSamples;
  final int activeStreams;

  factory MasterState.fromJson(Map<String, dynamic> json) => MasterState(
        rmsDb: _d(json['rms_db']),
        peakDb: _d(json['peak_db']),
        peakHoldDb: _d(json['peak_hold_db']),
        framesRendered: _i(json['frames_rendered']),
        clippedSamples: _i(json['clipped_samples']),
        activeStreams: _i(json['active_streams']),
      );
}

class ReceiverState {
  const ReceiverState({
    required this.datagrams,
    required this.bytes,
    required this.badPackets,
    required this.timeouts,
  });

  final int datagrams;
  final int bytes;
  final int badPackets;
  final int timeouts;

  factory ReceiverState.fromJson(Map<String, dynamic> json) => ReceiverState(
        datagrams: _i(json['datagrams']),
        bytes: _i(json['bytes']),
        badPackets: _i(json['bad_packets']),
        timeouts: _i(json['timeouts']),
      );
}

class StreamState {
  const StreamState({
    required this.id,
    required this.origin,
    required this.streamId,
    required this.label,
    required this.addr,
    required this.path,
    required this.mixCount,
    required this.codec,
    required this.active,
    required this.gainDb,
    required this.muted,
    required this.rmsDb,
    required this.peakDb,
    required this.peakHoldDb,
    required this.bufferMs,
    required this.targetMs,
    required this.bufferMaxMs,
    required this.ratio,
    required this.underruns,
    required this.underrunFrames,
    required this.bufferResets,
    required this.packets,
    required this.lostFrames,
    required this.plcFrames,
    required this.overflowPackets,
    required this.ageMs,
    required this.spectrum,
  });

  final int id;
  /// Sender uid as 16-char hex (JSON serializes u64 as a string).
  final String origin;
  /// 0xFFF0 is the hub's N-1 mix.
  final int streamId;
  final String label;
  final String addr;
  /// Carrier that delivered the newest frame *here*; empty until a packet lands.
  final String path;
  /// For a 0xFFF0 relay mix: how many nodes it folds in; 0 for device streams.
  final int mixCount;
  final String codec;
  final bool active;
  final double gainDb;
  final bool muted;
  final double rmsDb;
  final double peakDb;
  final double peakHoldDb;
  final double bufferMs;
  final double targetMs;
  /// Overflow-trim ceiling (2× target + 10 ms): above this the buffer is cut.
  final double bufferMaxMs;
  final double ratio;
  final int underruns;
  final int underrunFrames;
  final int bufferResets;
  final int packets;
  final int lostFrames;
  final int plcFrames;
  final int overflowPackets;
  final int ageMs;
  /// 32 log-spaced band levels (0..1) for the EQ display.
  final List<double> spectrum;

  factory StreamState.fromJson(Map<String, dynamic> json) => StreamState(
        id: _i(json['id']),
        origin: _s(json['origin']).toLowerCase(),
        streamId: _i(json['stream_id']),
        label: _s(json['label'], 'stream'),
        addr: _s(json['addr']),
        path: _s(json['path']),
        mixCount: _i(json['mix_count']),
        codec: _s(json['codec'], 'raw'),
        active: _b(json['active']),
        gainDb: _d(json['gain_db']),
        muted: _b(json['muted']),
        rmsDb: _d(json['rms_db']),
        peakDb: _d(json['peak_db']),
        peakHoldDb: _d(json['peak_hold_db']),
        bufferMs: _d(json['buffer_ms']),
        targetMs: _d(json['target_ms']),
        bufferMaxMs: _d(json['buffer_max_ms']),
        ratio: _d(json['ratio']),
        underruns: _i(json['underruns']),
        underrunFrames: _i(json['underrun_frames']),
        bufferResets: _i(json['buffer_resets']),
        packets: _i(json['packets']),
        lostFrames: _i(json['lost_frames']),
        plcFrames: _i(json['plc_frames']),
        overflowPackets: _i(json['overflow_packets']),
        ageMs: _i(json['age_ms']),
        spectrum: (json['spectrum'] as List? ?? const [])
            .map((value) => _d(value))
            .toList(growable: false),
      );
}

class CenterInfoState {
  const CenterInfoState({
    required this.bind,
    required this.mode,
    required this.device,
    required this.recordPath,
    required this.name,
  });

  final String bind;
  final String mode;
  final String device;
  final String? recordPath;
  final String name;

  factory CenterInfoState.fromJson(Map<String, dynamic> json) => CenterInfoState(
        bind: _s(json['bind']),
        mode: _s(json['mode'], 'playback'),
        device: _s(json['device']),
        recordPath: _sn(json['record_path']),
        name: _s(json['name'], 'Siren Center'),
      );

  String get port {
    final parts = bind.split(':');
    return parts.isEmpty ? '' : parts.last;
  }
}

class CenterState {
  const CenterState({
    required this.running,
    required this.stopping,
    required this.info,
    required this.targetMs,
    required this.masterGainDb,
    required this.seconds,
    required this.master,
    required this.streams,
    required this.receiver,
    required this.recordFrames,
    required this.recordSeconds,
    required this.error,
  });

  final bool running;
  final bool stopping;
  final CenterInfoState info;
  final double targetMs;
  final double masterGainDb;
  final double seconds;
  final MasterState master;
  final List<StreamState> streams;
  final ReceiverState receiver;
  final int recordFrames;
  final double recordSeconds;
  final String? error;

  factory CenterState.fromJson(Map<String, dynamic> json) => CenterState(
        running: _b(json['running']),
        stopping: _b(json['stopping']),
        info: CenterInfoState.fromJson(_map(json['info'])),
        targetMs: _d(json['target_ms']),
        masterGainDb: _d(json['master_gain_db']),
        seconds: _d(json['seconds']),
        master: MasterState.fromJson(_map(json['master'])),
        streams: _list(json['streams'], StreamState.fromJson),
        receiver: ReceiverState.fromJson(_map(json['receiver'])),
        recordFrames: _i(json['record_frames']),
        recordSeconds: _d(json['record_seconds']),
        error: _sn(json['error']),
      );
}

String formatDb(double db) => db <= -119.5 ? '−∞' : db.toStringAsFixed(1);

String formatCount(int value) {
  if (value >= 1000000000) return '${(value / 1000000000).toStringAsFixed(1)}G';
  if (value >= 1000000) return '${(value / 1000000).toStringAsFixed(1)}M';
  if (value >= 1000) return '${(value / 1000).toStringAsFixed(1)}k';
  return '$value';
}

String formatBytes(int value) {
  if (value >= 1024 * 1024 * 1024) {
    return '${(value / (1024 * 1024 * 1024)).toStringAsFixed(2)} GB';
  }
  if (value >= 1024 * 1024) {
    return '${(value / (1024 * 1024)).toStringAsFixed(1)} MB';
  }
  if (value >= 1024) return '${(value / 1024).toStringAsFixed(1)} KB';
  return '$value B';
}

String formatUptime(double seconds) {
  final total = seconds.floor();
  final hours = total ~/ 3600;
  final minutes = (total % 3600) ~/ 60;
  final secs = total % 60;
  if (hours > 0) return '${hours.toString().padLeft(2, '0')}:${minutes.toString().padLeft(2, '0')}:${secs.toString().padLeft(2, '0')}';
  return '${minutes.toString().padLeft(2, '0')}:${secs.toString().padLeft(2, '0')}';
}

String modeLabel(String mode) => switch (mode) {
      'record' => '录音',
      'silent' => '静默',
      _ => '播放',
    };

String sourceLabel(String source) => switch (source) {
      'loopback' => '系统内录',
      'microphone' => '麦克风',
      _ => '测试音',
    };


class MeshPeerState {
  const MeshPeerState({
    required this.name,
    required this.uid,
    required this.endpoint,
    required this.bt,
    required this.rttMs,
    required this.rttP99Ms,
    required this.lossPct,
  });

  final String name;
  /// Sender uid as 16-char hex; matches StreamState.origin.
  final String uid;
  final String endpoint;
  /// The peer's RFCOMM address. It says the Bluetooth path is *known*, not
  /// which carrier is currently delivering audio — both stacks send in parallel.
  final String? bt;
  /// Heartbeat path health to this node; null until the window has samples.
  final double? rttMs;
  final double? rttP99Ms;
  final double? lossPct;

  /// mDNS advertises the role suffix; panels show the plain device name.
  String get displayName {
    const suffix = ' (mesh)';
    final trimmed = name.trim();
    final plain = trimmed.endsWith(suffix)
        ? trimmed.substring(0, trimmed.length - suffix.length)
        : trimmed;
    return plain.isEmpty ? '节点' : plain;
  }

  factory MeshPeerState.fromJson(Map<String, dynamic> json) => MeshPeerState(
        name: _s(json['name'], 'peer'),
        uid: _s(json['uid']).toLowerCase(),
        endpoint: _s(json['endpoint']),
        bt: _sn(json['bt']),
        rttMs: _dn(json['rtt_ms']),
        rttP99Ms: _dn(json['rtt_p99_ms']),
        lossPct: _dn(json['loss_pct']),
      );
}

class BtLinkState {
  const BtLinkState({
    required this.enabled,
    required this.role,
    required this.state,
    required this.hubName,
    required this.hubAddr,
    required this.localAddr,
    required this.channel,
    required this.links,
    required this.lastError,
  });

  final bool enabled;
  final String role;
  final String state;
  final String? hubName;
  final String? hubAddr;
  final String? localAddr;
  final int channel;
  final int links;
  final String? lastError;

  factory BtLinkState.fromJson(Map<String, dynamic> json) => BtLinkState(
        enabled: _b(json['enabled']),
        role: _s(json['role'], 'off'),
        state: _s(json['state'], 'off'),
        hubName: _sn(json['hub_name']),
        hubAddr: _sn(json['hub_addr']),
        localAddr: _sn(json['local_addr']),
        channel: _i(json['channel']),
        links: _i(json['links']),
        lastError: _sn(json['last_error']),
      );

  String get label {
    if (!enabled) return '蓝牙关闭';
    final roleText = switch (role) {
      'hub' => '中心',
      'leaf' => '节点',
      _ => '',
    };
    final stateText = switch (state) {
      'connected' => '已连接',
      'listening' => '监听中',
      'connecting' => '连接中',
      'error' => '错误',
      _ => '等待',
    };
    final parts = [roleText, stateText].where((part) => part.isNotEmpty);
    final text = parts.join(' · ');
    return links > 0 ? '$text · $links 路' : text;
  }
}

class MeshState {
  const MeshState({
    required this.running,
    required this.stopping,
    required this.name,
    required this.nodeId,
    required this.seconds,
    required this.bind,
    required this.output,
    required this.peers,
    required this.topology,
    required this.bt,
    required this.senderFrames,
    required this.senderPackets,
    required this.senderBytes,
    required this.senderKbps,
    required this.senderRmsDb,
    required this.captureFrames,
    required this.captureDroppedFrames,
    required this.captureSilentFrames,
    required this.captureReadErrors,
    required this.captureBufferFrames,
    required this.center,
    required this.error,
  });

  final bool running;
  final bool stopping;
  final String name;
  final String nodeId;
  final double seconds;
  final String bind;
  final bool output;
  final List<MeshPeerState> peers;
  final List<MeshPeerState> topology;
  final BtLinkState bt;
  final int senderFrames;
  final int senderPackets;
  final int senderBytes;
  final double senderKbps;
  final double senderRmsDb;
  final int captureFrames;
  final int captureDroppedFrames;
  final int captureSilentFrames;
  final int captureReadErrors;
  final int captureBufferFrames;
  final CenterState center;
  final String? error;

  /// Human name for a stream: match its sender uid against known peers.
  String streamName(StreamState stream) {
    final mix = stream.streamId == 0xFFF0;
    final label = mix && stream.mixCount > 0 ? '混音 · 含 ${stream.mixCount} 路' : '混音';
    for (final peer in [...peers, ...topology]) {
      if (peer.uid.isNotEmpty && peer.uid == stream.origin) {
        final name = peer.displayName;
        return mix ? '$name · $label' : name;
      }
    }
    return mix ? label : stream.label;
  }

  factory MeshState.fromJson(Map<String, dynamic> json) => MeshState(
        running: _b(json['running']),
        stopping: _b(json['stopping']),
        name: _s(json['name'], 'mesh'),
        nodeId: _s(json['node_id']),
        seconds: _d(json['seconds']),
        bind: _s(json['bind']),
        output: _b(json['output']),
        peers: (json['peers'] as List? ?? const [])
            .whereType<Map>()
            .map((item) => MeshPeerState.fromJson(Map<String, dynamic>.from(item)))
            .toList(growable: false),
        topology: (json['topology'] as List? ?? const [])
            .whereType<Map>()
            .map((item) => MeshPeerState.fromJson(Map<String, dynamic>.from(item)))
            .toList(growable: false),
        bt: BtLinkState.fromJson(_map(json['bt'])),
        senderFrames: _i(json['sender']?['frames']),
        senderPackets: _i(json['sender']?['packets']),
        senderBytes: _i(json['sender']?['bytes']),
        senderKbps: _d(json['sender']?['kbps']),
        senderRmsDb: _d(json['sender']?['tx_rms_db']),
        captureFrames: _i(json['sender']?['capture_frames']),
        captureDroppedFrames: _i(json['sender']?['capture_dropped_frames']),
        captureSilentFrames: _i(json['sender']?['capture_silent_frames']),
        captureReadErrors: _i(json['sender']?['capture_read_errors']),
        captureBufferFrames: _i(json['sender']?['capture_buffer_frames']),
        center: CenterState.fromJson(_map(json['center'])),
        error: _sn(json['error']),
      );
}
