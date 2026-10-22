import { Button, Group } from "@mantine/core";
import { useDoc } from "../doc";
import { sameTarget, useNav, type Target } from "../nav";

/** Starting points of the document: its overview, the trailer, the catalog and Info. */
export function Landmarks() {
  const nav = useNav();
  const { summary: s } = useDoc();
  const items: [string, Target, string][] = [
    ["Overview", { kind: "document" }, "Info, what the catalog declares, XMP metadata"],
    ["Trailer", { kind: "trailer" }, "Merged trailer of all revisions"],
  ];
  if (s.root) items.push(["Catalog", { kind: "object", id: s.root }, "The document catalog (/Root)"]);
  if (s.info_dict) items.push(["Info", { kind: "object", id: s.info_dict }, "The document information dictionary"]);
  return (
    <Group gap={4} px="sm" py={6} style={{ borderBottom: "1px solid var(--mantine-color-default-border)" }}>
      {items.map(([label, t, title]) => (
        <Button
          key={label}
          size="compact-xs"
          variant={sameTarget(nav.target, t) ? "light" : "subtle"}
          title={title}
          onClick={() => nav.open(t)}
        >
          {label}
        </Button>
      ))}
    </Group>
  );
}
