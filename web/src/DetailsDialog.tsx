import { useEffect, useRef, type ReactNode } from "react";

export function DetailsDialog({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    function closeOnNavigation() {
      dialog?.close();
    }
    window.addEventListener("hashchange", closeOnNavigation);
    return () => window.removeEventListener("hashchange", closeOnNavigation);
  }, []);
  return (
    <dialog
      ref={ref}
      className="detail-dialog"
      aria-label={title}
      onClose={onClose}
    >
      <header className="section-header">
        <h2>{title}</h2>
        <button autoFocus onClick={() => ref.current?.close()}>
          关闭详情
        </button>
      </header>
      <div className="detail-body">{children}</div>
    </dialog>
  );
}
