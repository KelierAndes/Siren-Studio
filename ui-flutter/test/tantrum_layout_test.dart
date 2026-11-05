import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show FontLoader;
import 'package:flutter_test/flutter_test.dart';
import 'package:siren_ui/tantrum/controls.dart';
import 'package:siren_ui/tantrum/model.dart';
import 'package:siren_ui/tantrum/panels.dart';
import 'package:siren_ui/tantrum/tantrum_page.dart';

/// 新控制台是固定设计画布：任何 RenderFlex 溢出、控件互相重叠、
/// 或面板超出窗口，都必须在这里先失败，而不是等到真机上看见。
///
/// flutter_test 默认把每个字形画成方块（Ahem），丝印和读数会不会挤在一起
/// 完全看不出来。这里把 Windows 真实字体注册进 'Segoe UI' 这个 family，
/// 金标准里才是用户会看到的那张图；字体文件不存在就退回 Ahem。
Future<void> _loadRealFonts() async {
  final dir = Platform.environment['WINDIR'] ?? r'C:\Windows';
  for (final face in const ['segoeui.ttf', 'segoeuis.ttf', 'segoeuib.ttf', 'msyh.ttc']) {
    final file = File('$dir\\Fonts\\$face');
    if (!file.existsSync()) continue;
    final loader = FontLoader('Segoe UI')
      ..addFont(file.readAsBytes().then((b) => ByteData.view(b.buffer)));
    // 逐个注册：ttc 这类集合字体在测试引擎里可能直接解析失败，
    // 一个失败不该把整套金标准测试拖垮。
    try {
      await loader.load();
    } catch (_) {}
  }
}

void main() {
  setUpAll(_loadRealFonts);

  for (final size in const [
    Size(1280, 720),
    Size(1600, 900),
    Size(980, 640),
    Size(2560, 1440),
  ]) {
    testWidgets('面板在 ${size.width.toInt()}x${size.height.toInt()} 无溢出且不重叠', (tester) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);

      await tester.pumpWidget(MaterialApp(home: TantrumPage(model: TantrumModel())));
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
      expect(find.byType(TantrumKnob), findsNWidgets(5));

      final knobs = [
        for (var i = 0; i < 5; i++)
          tester.getRect(find.byKey(ValueKey('knob$i'))),
      ];
      for (var i = 0; i < knobs.length; i++) {
        for (var j = i + 1; j < knobs.length; j++) {
          expect(knobs[i].overlaps(knobs[j]), isFalse, reason: '旋钮 $i 与 $j 重叠：$knobs');
        }
      }
      // 读数条不能压到旋钮行
      final readout = tester.getRect(find.byType(TantrumSegPill));
      expect(readout.bottom <= knobs.first.top + 1, isTrue, reason: '读数条压到了旋钮行');
      // 面板整块落在窗口内
      final shell = tester.getRect(find.byType(TantrumShell));
      expect(shell.left >= 0 && shell.top >= 0, isTrue);
      expect(shell.right <= size.width && shell.bottom <= size.height, isTrue,
          reason: '面板超出窗口：$shell vs $size');
    });

    testWidgets('截图存档 ${size.width.toInt()}x${size.height.toInt()}', (tester) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);

      await tester.pumpWidget(MaterialApp(home: TantrumPage(model: TantrumModel())));
      await tester.pumpAndSettle();

      await expectLater(
        find.byType(TantrumPage),
        matchesGoldenFile('goldens/tantrum_${size.width.toInt()}x${size.height.toInt()}.png'),
      );
    });
  }

  testWidgets('ADV + 均衡 两个开关同时打开的形态', (tester) async {
    tester.view.physicalSize = const Size(1600, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    final model = TantrumModel()
      ..advanced = true
      ..screenMode = 1;
    await tester.pumpWidget(MaterialApp(home: TantrumPage(model: model)));
    await tester.pumpAndSettle();

    expect(find.byType(TantrumEqGraph), findsOneWidget);
    expect(find.byType(TantrumFader), findsNWidgets(4));
    expect(tester.takeException(), isNull);
    await expectLater(
      find.byType(TantrumPage),
      matchesGoldenFile('goldens/tantrum_adv_eq_on.png'),
    );
  });

  testWidgets('ADV 开关把右格栅换成四推子', (tester) async {
    tester.view.physicalSize = const Size(1600, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    await tester.pumpWidget(MaterialApp(home: TantrumPage(model: TantrumModel())));
    await tester.pumpAndSettle();

    expect(find.byType(TantrumGrille), findsOneWidget);
    expect(find.byType(TantrumFader), findsNothing);

    await tester.tap(find.byType(TantrumSwitch));
    await tester.pumpAndSettle();

    expect(find.byType(TantrumGrille), findsNothing);
    expect(find.byType(TantrumFader), findsNWidgets(4));
    expect(tester.takeException(), isNull);
  });

  testWidgets('屏模式胶囊把示波窗换成 EQ 图窗', (tester) async {
    tester.view.physicalSize = const Size(1600, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    await tester.pumpWidget(MaterialApp(home: TantrumPage(model: TantrumModel())));
    await tester.pumpAndSettle();

    expect(find.byType(TantrumScope), findsOneWidget);
    final pill = tester.getRect(find.byType(TantrumSegPill));
    await tester.tapAt(Offset(pill.right - pill.width / 4, pill.center.dy));
    await tester.pumpAndSettle();

    expect(find.byType(TantrumScope), findsNothing);
    expect(find.byType(TantrumEqGraph), findsOneWidget);
  });
}
