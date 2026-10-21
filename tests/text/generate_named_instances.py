"""Generate original named-instance fixtures with fonttools 4.60.1."""
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

from fontTools.designspaceLib import AxisDescriptor, DesignSpaceDocument, InstanceDescriptor, SourceDescriptor
from fontTools.ttLib import TTFont
from fontTools.varLib import build
from generate_fonts import write_font


class NamedInstanceFixtures:
    def build(self, destination, weight_axis):
        family = "Parity Multi Axis" if weight_axis else "Parity Width Axis"
        with TemporaryDirectory() as temporary:
            directory = Path(temporary)
            document = DesignSpaceDocument()
            axes = [("Weight", "wght", 400, 400, 700)] if weight_axis else []
            axes += [("Width", "wdth", 75, 100, 125)]
            for name, tag, minimum, default, maximum in axes:
                axis = AxisDescriptor()
                axis.name, axis.tag = name, tag
                axis.minimum, axis.default, axis.maximum = minimum, default, maximum
                document.addAxis(axis)
            for weight in ([400, 700] if weight_axis else [400]):
                for width in [75, 100, 125]:
                    filename = f"{weight}-{width}.ttf"
                    write_font(directory, filename, family, 500 + (weight - 400) // 3 + (width - 100) * 4)
                    font = TTFont(directory / filename, recalcTimestamp=False)
                    for table in font["cmap"].tables:
                        if table.isUnicode():
                            table.cmap.update({ord(c): "A" for c in "W0123456789Hx"})
                    font.save(directory / filename)
                    source = SourceDescriptor()
                    source.path = str(directory / filename)
                    source.location = {"Width": width}
                    if weight_axis:
                        source.location["Weight"] = weight
                    source.copyInfo = source.copyLib = source.copyFeatures = weight == 400 and width == 100
                    document.addSource(source)
            instances = [("Regular", 400, 100), ("Narrow", 400, 75), ("Wide", 700, 125)]
            if weight_axis:
                # Lowest Q16 bit is lost by an f32 intermediate at this weight.
                instances += [("Fractional", 550.125 + 1 / 65536, 87.5 + 1 / 65536)]
            for name, weight, width in instances:
                instance = InstanceDescriptor()
                instance.familyName, instance.styleName = family, name
                instance.location = {"Width": width}
                if weight_axis:
                    instance.location["Weight"] = weight
                document.addInstance(instance)
            variable, _, _ = build(document)
            variable.recalcTimestamp = False
            variable["head"].created = variable["head"].modified = 2082844800
            variable.save(destination)

    def run(self, directory):
        self.build(directory / "multi-axis.ttf", True)
        self.build(directory / "width-axis.ttf", False)
        default = TTFont(directory / "multi-axis.ttf", recalcTimestamp=False)
        default["fvar"].instances.clear()
        default.save(directory / "default-axis.ttf")


if __name__ == "__main__":
    NamedInstanceFixtures().run(Path(sys.argv[1]))
