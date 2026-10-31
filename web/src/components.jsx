import React, { useEffect, useId, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { X, Paperclip, Send, Check } from "lucide-react";
import { board, friendly, authGuard } from "./api";
import { useApp, useDraft } from "./context";
export function ErrorMessage({ error }) {
  return error ? (
    <p className="error" role="alert">
      {friendly(error)}
    </p>
  ) : null;
}
export function BusyButton({ busy, children, ...props }) {
  return (
    <button {...props} disabled={busy || props.disabled}>
      {busy ? "Working…" : children}
    </button>
  );
}
export function Modal({ title, children, onClose, wide = false }) {
  const ref = useRef(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current;
    dialog.showModal();
    const cancel = (e) => {
      e.preventDefault();
      onClose();
    };
    dialog.addEventListener("cancel", cancel);
    return () => {
      dialog.removeEventListener("cancel", cancel);
      dialog.close();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className={wide ? "wide-dialog" : ""}
      aria-labelledby={titleId}
    >
      <div className="modal-heading">
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="icon"
          aria-label="Close"
          onClick={onClose}
        >
          <X size={20} />
        </button>
      </div>
      {children}
    </dialog>
  );
}
export function Avatar({ person, small = false }) {
  const [failed, setFailed] = useState(null);
  const name = person?.name || person?.handle || "?";
  const initials = name
    .trim()
    .split(/[\s_-]+/)
    .map((part) => Array.from(part)[0] || "")
    .slice(0, 2)
    .join("")
    .toUpperCase();
  return (
    <span
      className={`avatar ${small ? "avatar-small" : ""}`}
      aria-hidden="true"
    >
      {person?.avatarUrl && failed !== person.avatarUrl ? (
        <img
          src={person.avatarUrl}
          alt=""
          onError={() => setFailed(person.avatarUrl)}
        />
      ) : (
        initials
      )}
    </span>
  );
}
export function AvatarEditor({ person }) {
  const app = useApp();
  const input = useRef(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [notice, setNotice] = useState("");
  async function save(file) {
    if (busy) return;
    setBusy(true);
    setError(null);
    setNotice("");
    let bitmap;
    const guard = authGuard();
    try {
      let data = null;
      if (file) {
        if (!file.type.startsWith("image/") || file.size > 10 * 1024 * 1024)
          throw new Error("Choose an image up to 10 MB.");
        bitmap = await createImageBitmap(file);
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 256;
        const size = Math.min(bitmap.width, bitmap.height);
        canvas
          .getContext("2d")
          .drawImage(
            bitmap,
            (bitmap.width - size) / 2,
            (bitmap.height - size) / 2,
            size,
            size,
            0,
            0,
            256,
            256,
          );
        data = canvas.toDataURL("image/png").split(",")[1];
      }
      guard();
      await board("set_avatar", { actor: person.id, data });
      await app.refresh();
      setNotice(file ? "Avatar updated." : "Default avatar restored.");
    } catch (e) {
      setError(e);
    } finally {
      bitmap?.close();
      setBusy(false);
      if (input.current) input.current.value = "";
    }
  }
  return (
    <div className="avatar-editor">
      <Avatar person={person} />
      <div className="actions">
        <input
          ref={input}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          hidden
          onChange={(e) => {
            if (e.target.files[0]) save(e.target.files[0]);
          }}
        />
        <BusyButton
          type="button"
          busy={busy}
          onClick={() => input.current.click()}
        >
          Change avatar
        </BusyButton>
        {person.customAvatar && (
          <button type="button" disabled={busy} onClick={() => save(null)}>
            Remove avatar
          </button>
        )}
      </div>
      <small className="muted">
        Images are cropped to a square. Visible to everyone in this workspace.
      </small>
      <ErrorMessage error={error} />
      {notice && (
        <p className="success" role="status">
          {notice}
        </p>
      )}
    </div>
  );
}
export function Person({ id, mention = false }) {
  const { people, openPerson } = useApp();
  const p = people.find((p) => p.id === id);
  return p ? (
    <button
      type="button"
      className="person-link"
      onClick={() => openPerson(p.id)}
    >
      {!mention && <Avatar person={p} small />}
      {mention ? "@" + p.handle : p.name}
    </button>
  ) : (
    <span>{id || "Unassigned"}</span>
  );
}
function highlightMentions(node, people) {
  return React.Children.map(node, (child) => {
    if (typeof child === "string") {
      const result = [];
      let at = 0;
      for (const match of child.matchAll(/(^|[\s([{])@([a-zA-Z0-9_.-]+)/g)) {
        const start = match.index + match[1].length;
        result.push(child.slice(at, start));
        let handle = match[2];
        let person = people.find((p) => p.handle === handle.toLowerCase());
        while (!person && handle.endsWith(".")) {
          handle = handle.slice(0, -1);
          person = people.find((p) => p.handle === handle.toLowerCase());
        }
        result.push(
          person ? (
            <Person key={start} id={person.id} mention />
          ) : (
            `@${match[2]}`
          ),
        );
        if (person) result.push(match[2].slice(handle.length));
        at = match.index + match[0].length;
      }
      result.push(child.slice(at));
      return result;
    }
    if (React.isValidElement(child) && child.type !== "code")
      return React.cloneElement(child, {
        children: highlightMentions(child.props.children, people),
      });
    return child;
  });
}
function MarkdownParagraph({ children }) {
  const { people } = useApp();
  return <p>{highlightMentions(children, people)}</p>;
}
function MarkdownListItem({ children }) {
  const { people } = useApp();
  return <li>{highlightMentions(children, people)}</li>;
}
const markdownComponents = {
  p: MarkdownParagraph,
  li: MarkdownListItem,
  a: ({ children, href }) => (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
  td: ({ style, ...props }) => <td {...props} />,
  th: ({ style, ...props }) => <th {...props} />,
};
export function RichText({ text = "" }) {
  return (
    <div className="markdown">
      <Markdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
        {text}
      </Markdown>
    </div>
  );
}
export function Attachments({ files = [] }) {
  return (
    <div className="attachments">
      {files.map((file) => (
        <a
          key={file.id}
          href={`/api/attachments/${encodeURIComponent(file.id)}`}
          target="_blank"
          rel="noopener noreferrer"
        >
          {["image/png", "image/jpeg", "image/gif", "image/webp"].includes(
            file.mime,
          ) && (
            <img
              src={`/api/attachments/${encodeURIComponent(file.id)}`}
              alt={file.name}
              loading="lazy"
            />
          )}
          <span>
            <Paperclip size={13} />
            {file.name}
            <small>{Math.ceil(file.size / 1024)} KB</small>
          </span>
        </a>
      ))}
    </div>
  );
}
export function MentionInput({
  value,
  onChange,
  onSubmit,
  disabled,
  label = "Message",
  placeholder,
  rows = 3,
}) {
  const { people } = useApp();
  const input = useRef(null);
  const [query, setQuery] = useState(null);
  const [index, setIndex] = useState(0);
  const listId = useId();
  const composing = useRef(false);
  const matches =
    query === null
      ? []
      : people
          .filter((p) => p.active !== false)
          .filter((p) =>
            `${p.handle} ${p.name}`
              .toLowerCase()
              .includes(query.text.toLowerCase()),
          )
          .slice(0, 8);
  function find(value, position) {
    const prefix = value.slice(0, position);
    const match = prefix.match(/(?:^|[\s([{])@([a-zA-Z0-9_.-]*)$/);
    setQuery(
      match
        ? {
            text: match[1],
            start: position - match[1].length - 1,
            end: position,
          }
        : null,
    );
    setIndex(0);
  }
  function insert(p) {
    const next =
      value.slice(0, query.start) + `@${p.handle} ` + value.slice(query.end);
    const position = query.start + p.handle.length + 2;
    onChange(next);
    setQuery(null);
    requestAnimationFrame(() => {
      input.current?.focus();
      input.current?.setSelectionRange(position, position);
    });
  }
  function key(e) {
    if (e.nativeEvent.isComposing || composing.current || e.keyCode === 229)
      return;
    if (matches.length && query) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setIndex(
          (i) =>
            (i + (e.key === "ArrowDown" ? 1 : -1) + matches.length) %
            matches.length,
        );
        return;
      }
      if ((e.key === "Enter" && !e.shiftKey) || e.key === "Tab") {
        e.preventDefault();
        insert(matches[Math.min(index, matches.length - 1)]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setQuery(null);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (!disabled) onSubmit?.();
    }
  }
  return (
    <div className="mention-input">
      <textarea
        ref={input}
        value={value}
        disabled={disabled}
        rows={rows}
        aria-label={label}
        placeholder={placeholder}
        aria-autocomplete="list"
        aria-expanded={matches.length > 0}
        aria-controls={matches.length ? listId : undefined}
        aria-activedescendant={
          matches.length ? `${listId}-${index}` : undefined
        }
        maxLength={200000}
        onCompositionStart={() => (composing.current = true)}
        onCompositionEnd={() => (composing.current = false)}
        onKeyDown={key}
        onChange={(e) => {
          onChange(e.target.value);
          find(e.target.value, e.target.selectionStart);
        }}
        onClick={(e) => find(value, e.target.selectionStart)}
        onBlur={() => setTimeout(() => setQuery(null), 120)}
      />
      {matches.length > 0 && (
        <div
          className="mention-menu"
          role="listbox"
          id={listId}
          aria-label="Mention a person or agent"
        >
          {matches.map((p, i) => (
            <button
              id={`${listId}-${i}`}
              key={p.id}
              type="button"
              role="option"
              aria-selected={i === index}
              className={i === index ? "active" : ""}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => insert(p)}
            >
              <Avatar person={p} small />
              <span>
                {p.name}
                <small>
                  @{p.handle} · {p.kind}
                </small>
              </span>
              {i === index && <Check size={14} />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
export function Composer({
  draftKey,
  onSubmit,
  title = false,
  label = "Message",
  buttonText = "Send",
  placeholder = "Write a message… Type @ to mention someone.",
}) {
  const [text, setText] = useDraft(draftKey);
  const [heading, setHeading] = useDraft(draftKey + ":title");
  const [files, setFiles] = useDraft(draftKey + ":files", []);
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [uploading, setUploading] = useState(false);
  const fileInput = useRef(null);
  const request = useRef(null);
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );
  async function upload(chosen) {
    if (!chosen.length) return;
    if (files.length + chosen.length > 10) {
      setError(new Error("At most 10 attachments per post."));
      return;
    }
    setUploading(true);
    setError(null);
    try {
      for (const file of chosen) {
        if (!alive.current) return;
        if (file.size > 20 * 1024 * 1024)
          throw new Error("Files must be 20 MB or smaller.");
        const data = await new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(String(reader.result).split(",")[1]);
          reader.onerror = reject;
          reader.readAsDataURL(file);
        });
        if (!alive.current) return;
        const saved = await board("upload_attachment", {
          name: file.name,
          mime: file.type || "application/octet-stream",
          data,
        });
        setFiles((old) => [...old, saved]);
      }
    } catch (e) {
      setError(e);
    } finally {
      setUploading(false);
      if (fileInput.current) fileInput.current.value = "";
    }
  }
  async function send() {
    if (busyRef.current || uploading || (!text.trim() && !files.length)) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    request.current ??= crypto.randomUUID();
    const sent = text;
    try {
      await onSubmit({
        body: text,
        title: heading,
        attachments: files.map((f) => f.id),
        requestId: request.current,
      });
      setText((old) => (old === sent ? "" : old));
      setHeading("");
      setFiles([]);
      request.current = null;
    } catch (e) {
      setError(e);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }
  return (
    <form
      className="composer-card"
      onSubmit={(e) => {
        e.preventDefault();
        send();
      }}
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) e.preventDefault();
      }}
      onDrop={(e) => {
        if (e.dataTransfer.files.length) {
          e.preventDefault();
          upload([...e.dataTransfer.files]);
        }
      }}
      onPaste={(e) => {
        const incoming = [...e.clipboardData.files];
        if (incoming.length) {
          e.preventDefault();
          upload(incoming);
        }
      }}
    >
      {title && (
        <input
          aria-label="Topic title"
          placeholder="Topic title (optional)"
          maxLength={500}
          value={heading}
          onChange={(e) => {
            setHeading(e.target.value);
            request.current = null;
          }}
        />
      )}
      <MentionInput
        value={text}
        onChange={(v) => {
          setText(v);
          request.current = null;
        }}
        onSubmit={send}
        disabled={busy}
        label={label}
        placeholder={placeholder}
      />
      {files.length > 0 && (
        <div className="pending-files">
          {files.map((f) => (
            <span key={f.id}>
              <Paperclip size={13} />
              {f.name}
              <button
                type="button"
                aria-label={`Remove ${f.name}`}
                onClick={() =>
                  setFiles((old) => old.filter((x) => x.id !== f.id))
                }
              >
                <X size={13} />
              </button>
            </span>
          ))}
        </div>
      )}
      <ErrorMessage error={error} />
      <div className="composer-toolbar">
        <div>
          <input
            ref={fileInput}
            type="file"
            multiple
            hidden
            onChange={(e) => upload([...e.target.files])}
          />
          <button
            type="button"
            className="icon"
            aria-label="Attach files"
            onClick={() => fileInput.current.click()}
            disabled={uploading || busy}
          >
            <Paperclip size={18} />
          </button>
          <span className="muted">
            {uploading
              ? "Uploading…"
              : "Enter to send · Shift+Enter for a new line"}
          </span>
        </div>
        <BusyButton
          type="submit"
          className="primary"
          busy={busy}
          disabled={uploading}
        >
          <Send size={15} />
          {buttonText}
        </BusyButton>
      </div>
    </form>
  );
}
