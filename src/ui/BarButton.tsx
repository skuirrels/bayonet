import { forwardRef, type ButtonHTMLAttributes } from "react";
import clsx from "clsx";
import { tooltipAttrs } from "./Tooltip";
import styles from "./BarButton.module.css";

export interface BarButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "title"> {
  /** `status`: status bar widget; `title`: main toolbar widget. */
  variant?: "status" | "title";
  mono?: boolean;
  /** Tooltip text (also the accessible name unless `aria-label` is set). */
  tooltip?: string;
  shortcut?: string;
  emphasis?: boolean;
}

/** Text button for status bar / title bar widgets contributed by features. */
export const BarButton = forwardRef<HTMLButtonElement, BarButtonProps>(function BarButton(
  { variant = "status", mono, tooltip, shortcut, emphasis, className, type = "button", children, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      className={clsx(styles.button, styles[variant], mono && styles.mono, emphasis && styles.emphasis, className)}
      {...tooltipAttrs(tooltip, shortcut)}
      {...rest}
    >
      {children}
    </button>
  );
});
