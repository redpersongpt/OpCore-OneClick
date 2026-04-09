import { useEffect, useCallback, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'motion/react';
import { X } from 'lucide-react';

export interface ModalProps {
  open: boolean;
  onClose: () => void;
  title?: ReactNode;
  footer?: ReactNode;
  /** Max width class — default 'max-w-md' */
  width?: string;
  /** Remove default padding from body */
  noPadding?: boolean;
  children: ReactNode;
}

export function Modal({
  open,
  onClose,
  title,
  footer,
  width = 'max-w-md',
  noPadding = false,
  children,
}: ModalProps) {
  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    },
    [onClose],
  );

  useEffect(() => {
    if (open) {
      document.addEventListener('keydown', handleKeyDown);
      return () => document.removeEventListener('keydown', handleKeyDown);
    }
  }, [open, handleKeyDown]);

  return createPortal(
    <AnimatePresence>
      {open && (
        <>
          {/* Backdrop */}
          <motion.div
            key="modal-backdrop"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15, ease: 'easeOut' }}
            className="fixed inset-0 z-50 bg-black/60"
            onClick={onClose}
            aria-hidden
          />

          {/* Dialog */}
          <div
            className="fixed inset-0 z-50 flex items-center justify-center p-4"
            role="dialog"
            aria-modal="true"
          >
            <motion.div
              key="modal-panel"
              initial={{ opacity: 0, scale: 0.97, y: 4 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.97, y: 4 }}
              transition={{ duration: 0.2, ease: 'easeOut' }}
              className={[
                'relative w-full rounded-lg',
                'bg-[--surface-2] border border-[--border]',
                'flex flex-col',
                width,
              ].join(' ')}
              onClick={(e) => e.stopPropagation()}
            >
              {/* Header */}
              {title && (
                <div className="flex items-center justify-between px-5 py-4 border-b border-[--border-subtle]">
                  <span className="text-[0.9375rem] font-semibold text-[--text-primary] leading-snug">
                    {title}
                  </span>
                  <button
                    onClick={onClose}
                    className="oc-focus-ring ml-3 shrink-0 text-[--text-tertiary] hover:text-[--text-secondary] transition-colors rounded p-0.5"
                    aria-label="Close"
                  >
                    <X size={15} />
                  </button>
                </div>
              )}

              {/* Body */}
              <div className={`${noPadding ? '' : 'px-5 py-4'} max-h-[70vh] overflow-y-auto`}>
                {children}
              </div>

              {/* Footer */}
              {footer && (
                <div className="px-5 py-4 border-t border-[--border-subtle] flex items-center justify-end gap-2">
                  {footer}
                </div>
              )}
            </motion.div>
          </div>
        </>
      )}
    </AnimatePresence>,
    document.body,
  );
}
