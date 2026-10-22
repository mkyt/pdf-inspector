import { UnstyledButton } from "@mantine/core";
import type { ReactNode } from "react";
import classes from "./Row.module.css";

export function Row({ active, onClick, children }: { active?: boolean; onClick?: () => void; children: ReactNode }) {
  return (
    <UnstyledButton className={classes.row} data-active={active || undefined} onClick={onClick}>
      {children}
    </UnstyledButton>
  );
}
