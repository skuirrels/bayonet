/** What connecting to an SSH host may ask before a remote attach: trust in its key, a password. */
import { useState } from "react";
import { tx } from "@/platform/i18n";
import { Input } from "@/ui";
import { ask, confirm } from "@/workbench/messages";
import styles from "./Overlays.module.css";

export function confirmHostKey(message: string): Promise<boolean> {
  return confirm({
    title: "Unknown SSH Host",
    icon: "warning",
    message: `${message}. ${tx("The authenticity of the host cannot be established. Trust it and add the key to ~/.ssh/known_hosts?")}`,
    ok: "Trust and Connect",
  });
}

function SecretField({ message, onChange }: { message: string; onChange: (secret: string) => void }) {
  const [secret, setSecret] = useState("");
  return (
    <div className={styles.field}>
      <span>{message}</span>
      <Input
        type="password"
        data-autofocus=""
        aria-label="Password or passphrase"
        value={secret}
        onChange={(e) => {
          setSecret(e.target.value);
          onChange(e.target.value);
        }}
      />
    </div>
  );
}

/** Asks for the SSH password or key passphrase (`message`: the backend's error); null when cancelled. */
export async function askSshSecret(message: string, save: boolean): Promise<{ secret: string; save: boolean } | null> {
  let secret = "";
  const answer = await ask({
    title: "SSH Authentication",
    body: <SecretField message={message} onChange={(s) => (secret = s)} />,
    buttons: [{ id: "ok", label: "Connect", primary: true }],
    checkbox: "Save",
    checked: save,
    width: 420,
  });
  return answer.button === "ok" && secret ? { secret, save: answer.checked } : null;
}
