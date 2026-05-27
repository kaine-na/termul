import { getCurrentWindow } from '@tauri-apps/api/window'
import { Check, Copy, Globe, Minus, Square, X } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useRemoteStore } from '@/stores/remote-store'

const focusableButtonClass =
  'h-full px-3 hover:bg-secondary inline-flex items-center focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-inset'

export function TauriTitleBar(): React.JSX.Element {
  const [isMaximized, setIsMaximized] = useState(false)
  const appWindow = getCurrentWindow()

  // Remote server state
  const remoteIsRunning = useRemoteStore((s) => s.isRunning)
  const remotePort = useRemoteStore((s) => s.port)
  const remoteUrl = useRemoteStore((s) => s.url)
  const remoteStop = useRemoteStore((s) => s.stop)
  const remoteRefreshStatus = useRemoteStore((s) => s.refreshStatus)
  const [urlCopied, setUrlCopied] = useState(false)

  // Check remote status on mount
  useEffect(() => {
    void remoteRefreshStatus()
  }, [remoteRefreshStatus])

  const checkMaximized = useCallback(async () => {
    const maximized = await appWindow.isMaximized()
    setIsMaximized(maximized)
  }, [appWindow])

  useEffect(() => {
    let mounted = true
    checkMaximized()

    // Listen for resize events to track maximize state
    let unlisten: (() => void) | undefined
    appWindow.onResized(() => {
      checkMaximized()
    }).then((fn) => {
      if (mounted) {
        unlisten = fn
      } else {
        // Component already unmounted, clean up immediately
        fn()
      }
    })

    return () => {
      mounted = false
      unlisten?.()
    }
  }, [appWindow, checkMaximized])

  const handleCopyUrl = async (e: React.MouseEvent) => {
    e.stopPropagation()
    if (remoteUrl) {
      await navigator.clipboard.writeText(remoteUrl)
      setUrlCopied(true)
      setTimeout(() => setUrlCopied(false), 2000)
    }
  }

  const handleStop = async (e: React.MouseEvent) => {
    e.stopPropagation()
    await remoteStop()
  }

  return (
    <header
      className="h-8 flex items-center justify-between bg-card border-b border-border select-none shrink-0"
      data-tauri-drag-region
    >
      <span
        className="text-xs font-semibold text-muted-foreground tracking-wider uppercase px-3"
        data-tauri-drag-region
      >
        termul
      </span>

      {/* Remote Live Indicator — centered between title and window controls */}
      <div className="flex-1 flex justify-center" data-tauri-drag-region>
        {remoteIsRunning && (
          <Tooltip>
            <TooltipTrigger asChild>
              <div className="flex items-center gap-1.5 px-2.5 py-0.5 rounded-full bg-emerald-500/15 border border-emerald-500/30 cursor-pointer hover:bg-emerald-500/25 transition-colors">
                <span className="relative flex h-2 w-2">
                  <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75" />
                  <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-500" />
                </span>
                <Globe size={12} className="text-emerald-400" />
                <span className="text-[11px] font-medium text-emerald-400">
                  LIVE :{remotePort}
                </span>
              </div>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="max-w-xs">
              <div className="space-y-2">
                <div className="flex items-center gap-2">
                  <span className="relative flex h-2 w-2">
                    <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75" />
                    <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-500" />
                  </span>
                  <p className="font-semibold text-sm">Remote Terminal Active</p>
                </div>
                <p className="text-xs opacity-80 break-all font-mono leading-relaxed">{remoteUrl}</p>
                <div className="flex gap-2 pt-1">
                  <button
                    type="button"
                    onClick={handleCopyUrl}
                    className="flex items-center gap-1.5 text-xs px-2.5 py-1.5 bg-white/10 hover:bg-white/20 rounded-md transition-colors"
                  >
                    {urlCopied ? <Check size={12} /> : <Copy size={12} />}
                    {urlCopied ? 'Copied!' : 'Copy URL'}
                  </button>
                  <button
                    type="button"
                    onClick={handleStop}
                    className="text-xs px-2.5 py-1.5 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-md transition-colors"
                  >
                    Stop Server
                  </button>
                </div>
              </div>
            </TooltipContent>
          </Tooltip>
        )}
      </div>

      <div className="flex items-center h-full">
        <button
          type="button"
          onClick={() => appWindow.minimize()}
          className={focusableButtonClass}
          title="Minimize"
          aria-label="Minimize window"
          data-press-feedback="off"
        >
          <Minus size={16} />
        </button>

        <button
          type="button"
          onClick={() => appWindow.toggleMaximize()}
          className={focusableButtonClass}
          title={isMaximized ? 'Restore' : 'Maximize'}
          aria-label={isMaximized ? 'Restore window' : 'Maximize window'}
          data-press-feedback="off"
        >
          {isMaximized ? <Copy size={14} /> : <Square size={14} />}
        </button>

        <button
          type="button"
          onClick={() => appWindow.close()}
          className="h-full px-3 hover:bg-red-500/90 hover:text-white inline-flex items-center focus:outline-none focus-visible:ring-2 focus-visible:ring-red-500"
          title="Close"
          aria-label="Close window"
          data-press-feedback="off"
        >
          <X size={16} />
        </button>
      </div>
    </header>
  )
}
