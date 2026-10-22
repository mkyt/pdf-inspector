import { Anchor, Text } from "@mantine/core";
import { destText, linkText } from "../format";
import { useNav } from "../nav";
import type { LinkTarget } from "../protocol/generated";

/** Where a link goes; a page of this document opens on click. */
export function LinkView({ link }: { link: LinkTarget }) {
  const nav = useNav();
  if (link.kind === "dest" && link.dest.page_index !== null) {
    const index = link.dest.page_index;
    return (
      <Anchor size="xs" onClick={() => nav.open({ kind: "page", index })}>
        {destText(link.dest)}
      </Anchor>
    );
  }
  if (link.kind === "uri") {
    return (
      <Anchor size="xs" href={link.uri} target="_blank" rel="noopener noreferrer" style={{ wordBreak: "break-all" }}>
        {link.uri}
      </Anchor>
    );
  }
  return (
    <Text span size="xs" c={link.kind === "dest" && link.dest.error ? "red" : undefined}>
      {linkText(link)}
    </Text>
  );
}
