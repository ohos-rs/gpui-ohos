"""Regenerate tiny, original test fonts with fonttools 4.60.1 (not a runtime dependency)."""
from pathlib import Path
from tempfile import TemporaryDirectory

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.designspaceLib import DesignSpaceDocument, AxisDescriptor, SourceDescriptor, InstanceDescriptor
from fontTools.varLib import build
from fontTools.ttLib import TTFont
from fontTools.colorLib.builder import buildCOLR, buildCPAL


def write_font(directory, filename, family, width, fallback=False, bold=False):
    names = [".notdef", "space", "A", "B"]
    cmap = {32: "space", 65: "A", 66: "B"}
    if fallback:
        names.extend(["omega", "acute", "omega_ligature"])
        cmap.update({0x3A9: "omega", 0x301: "acute"})
    builder = FontBuilder(1000, isTTF=True)
    builder.setupGlyphOrder(names)
    builder.setupCharacterMap(cmap)
    glyphs = {}
    for name in names:
        pen = TTGlyphPen(None)
        if name != "space":
            pen.moveTo((50, 0))
            pen.lineTo((width - 50, 0))
            pen.lineTo((width - 50, 700))
            pen.lineTo((50, 700))
            pen.closePath()
        glyphs[name] = pen.glyph()
    builder.setupGlyf(glyphs)
    builder.setupHorizontalMetrics({name: (width, 50) for name in names})
    builder.setupHorizontalHeader(ascent=800, descent=-200)
    style = "Bold" if bold else "Regular"
    builder.setupNameTable({
        "familyName": family, "styleName": style, "uniqueFontIdentifier": family + style,
        "fullName": family + " " + style, "psName": family.replace(" ", "") + "-" + style,
        "version": "Version 1.000", "licenseDescription": "Original GPUI test fixture; MIT OR Apache-2.0",
    })
    builder.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200,
                     usWeightClass=700 if bold else 400, fsSelection=0x20 if bold else 0x40)
    builder.setupPost()
    builder.setupMaxp()
    builder.font["head"].macStyle = 1 if bold else 0
    builder.font["head"].created = builder.font["head"].modified = 2082844800
    builder.font.recalcTimestamp = False
    if fallback:
        addOpenTypeFeaturesFromString(builder.font, "feature liga { sub omega omega by omega_ligature; } liga;")
    builder.save(directory / filename)


if __name__ == "__main__":
    directory = Path(__file__).resolve().parent / "fonts"
    directory.mkdir(exist_ok=True)
    write_font(directory, "primary.ttf", "Parity Primary", 500)
    write_font(directory, "primary-bold.ttf", "Parity Primary", 600, bold=True)
    write_font(directory, "first.ttf", "Parity First", 700, fallback=True)
    write_font(directory, "second.ttf", "Parity Second", 900, fallback=True)
    write_font(directory, "new.ttf", "Parity New", 1100, fallback=True)
    # Distinct glyph IDs expose direction changes without installed fonts.
    write_font(directory, "direction.ttf", "Parity Direction", 500, fallback=True)
    direction = TTFont(directory / "direction.ttf", recalcTimestamp=False)
    direction["hhea"].ascent = 1100
    direction["OS/2"].sTypoAscender = 1100
    direction["OS/2"].usWinAscent = 1100
    for table in direction["cmap"].tables:
        if table.isUnicode():
            table.cmap.update({0x05D0: "A", 0x05D1: "B", 0x05D2: "omega", 0x0644: "A", 0x0627: "B", 0x0628: "omega", 0x0915: "A", 0x093F: "B"})
    addOpenTypeFeaturesFromString(direction, "languagesystem DFLT dflt; languagesystem deva dflt; feature liga { sub omega omega by omega_ligature; } liga;")
    direction.save(directory / "direction.ttf")
    write_font(directory, "metrics.ttf", "Parity Metrics", 500, fallback=True)
    metrics = TTFont(directory / "metrics.ttf", recalcTimestamp=False)
    metrics["hhea"].lineGap = 450
    metrics["OS/2"].sTypoLineGap = 450
    metrics["hmtx"].metrics["omega"] = (900, 50)
    for table in metrics["cmap"].tables:
        if table.isUnicode():
            table.cmap.update({ord(c): "A" for c in "0123456789Hx"})
            table.cmap.update({ord("W"): "omega", 0x6C34: "omega"})
    metrics.save(directory / "metrics.ttf")
    write_font(directory, "positioned.ttf", "Parity Positioned", 500)
    positioned = TTFont(directory / "positioned.ttf", recalcTimestamp=False)
    addOpenTypeFeaturesFromString(positioned, "feature kern { pos A <120 80 0 0>; } kern;")
    positioned.save(directory / "positioned.ttf")
    write_font(directory, "spaced.ttf", "Parity Spaced", 500, fallback=True)
    spaced = TTFont(directory / "spaced.ttf", recalcTimestamp=False)
    for table in spaced["cmap"].tables:
        if table.isUnicode():
            table.cmap.update({ord("f"): "A", ord("i"): "B", ord("l"): "B", ord("s"): "A", ord("t"): "B"})
    addOpenTypeFeaturesFromString(spaced, "feature liga { sub A space B by omega_ligature; sub A B by omega_ligature; } liga;")
    spaced.save(directory / "spaced.ttf")
    write_font(directory, "replacement.ttf", "Parity Replacement", 500)
    replacement = TTFont(directory / "replacement.ttf", recalcTimestamp=False)
    for table in replacement["cmap"].tables:
        if table.isUnicode():
            table.cmap[0xFFFD] = "B"
    # A distinct UVS glyph proves terminal selectors are used for face choice,
    # then stripped before shaping, as in Ghostty's run iterator.
    from fontTools.ttLib.tables._c_m_a_p import CmapSubtable
    selector = CmapSubtable.newSubtable(14)
    selector.platformID, selector.platEncID, selector.language = 0, 5, 0
    selector.cmap = {}
    selector.uvsDict = {0xFE0E: [(65, "B")]}
    replacement["cmap"].tables.append(selector)
    replacement.save(directory / "replacement.ttf")
    for color in [False, True]:
        filename = "emoji-color.ttf" if color else "emoji-text.ttf"
        write_font(directory, filename, "Parity Emoji Color" if color else "Parity Emoji Text", 800)
        emoji = TTFont(directory / filename, recalcTimestamp=False)
        for table in emoji["cmap"].tables:
            if table.isUnicode():
                table.cmap.update({0x2764: "A", 0x2600: "A", 0x270C: "A"})
        # Include a format-12 cmap for a supplementary-plane default emoji.
        from fontTools.ttLib.tables._c_m_a_p import CmapSubtable
        cmap = CmapSubtable.newSubtable(12)
        cmap.platformID, cmap.platEncID, cmap.language = 3, 10, 0
        cmap.cmap = {32: "space", 65: "A", 66: "B", 0x301: "B", 0x2764: "A", 0x2600: "A", 0x270C: "A", 0x1F600: "A", 0x1F469: "A"}
        if color:
            cmap.cmap[0x1F4BB] = "A"
        emoji["cmap"].tables.append(cmap)
        if color:
            emoji["COLR"] = buildCOLR({"A": [("B", 0)]}, version=0)
            emoji["CPAL"] = buildCPAL([[(1.0, 0.25, 0.0, 1.0)]])
        emoji.save(directory / filename)
    with TemporaryDirectory() as temporary:
        temporary = Path(temporary)
        write_font(temporary, "regular.ttf", "Parity Variable", 500)
        write_font(temporary, "bold.ttf", "Parity Variable", 600, bold=True)
        document = DesignSpaceDocument()
        axis = AxisDescriptor()
        axis.name, axis.tag, axis.minimum, axis.default, axis.maximum = "Weight", "wght", 400, 400, 700
        document.addAxis(axis)
        for weight, filename in [(400, "regular.ttf"), (700, "bold.ttf")]:
            source = SourceDescriptor()
            source.path = str(temporary / filename)
            source.location = {"Weight": weight}
            source.copyInfo = source.copyLib = source.copyFeatures = weight == 400
            document.addSource(source)
            instance = InstanceDescriptor()
            instance.familyName = "Parity Variable"
            instance.styleName = "Bold" if weight == 700 else "Regular"
            instance.location = {"Weight": weight}
            document.addInstance(instance)
        variable, _, _ = build(document)
        variable.recalcTimestamp = False
        variable["head"].created = variable["head"].modified = 2082844800
        variable.save(directory / "variable.ttf")
