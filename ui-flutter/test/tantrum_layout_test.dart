import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:siren_ui/tantrum/controls.dart';
import 'package:siren_ui/tantrum/model.dart';
import 'package:siren_ui/tantrum/panels.dart';
import 'package:siren_ui/tantrum/tantrum_page.dart';

/// 新控制台是固定设计画布：任何 RenderFlex 溢出、控件互相重叠、
/// 或面板超出窗口，都必须在这里先失败，而不是等到真机上看见。
void main() {
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
