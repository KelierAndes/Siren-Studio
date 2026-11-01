import 'package:flutter_test/flutter_test.dart';
import 'package:siren_ui/models.dart';

void main() {
  test('stream keeps the sender uid as hex and recognises the mix', () {
    final stream = StreamState.fromJson({
      'id': 1,
      'origin': '48A168CB52F1D8E7',
      'stream_id': 0xFFF0,
      'label': '48a168cb52f1d8e7#65520',
      'addr': '127.0.0.1:1',
      'path': 'bt',
      'codec': 'opus',
      'active': true,
      'gain_db': 0,
      'muted': false,
      'rms_db': -15,
      'peak_db': -12,
      'peak_hold_db': -12,
      'buffer_ms': 100,
      'target_ms': 250,
      'ratio': 1,
      'underruns': 0,
      'underrun_frames': 0,
      'buffer_resets': 0,
      'packets': 10,
      'lost_frames': 0,
      'plc_frames': 0,
      'overflow_packets': 0,
      'age_ms': 0,
    });
    expect(stream.origin, '48a168cb52f1d8e7');
    expect(stream.streamId, 0xFFF0);
  });

  test('peer uid is normalised to lower-case hex', () {
    final peer = MeshPeerState.fromJson({
      'name': 'ADMINISTRATOR',
      'uid': '48A168CB52F1D8E7',
      'endpoint': '192.168.2.107:47900',
      'bt': null,
    });
    expect(peer.uid, '48a168cb52f1d8e7');
  });
}
