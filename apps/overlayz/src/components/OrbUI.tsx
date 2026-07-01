import { Orb, AgentState } from './Orb'
import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

interface OrbUIProps {
  inputVolume: number
  outputVolume: number
  agentState: AgentState
  vadProgress: number
  audioEnabled: boolean
  onToggleAudio: () => Promise<void>
}

interface OrbColors {
  listening: [string, string, string]
  thinking: [string, string, string]
  talking: [string, string, string]
}

export function OrbUI({ inputVolume, outputVolume, agentState, vadProgress, audioEnabled, onToggleAudio }: OrbUIProps) {
  const radius = 62;
  const circumference = 2 * Math.PI * radius;
  const dashOffset = circumference * vadProgress;

  const [themeColors, setThemeColors] = useState<OrbColors>({
    listening: ['#CADCFC', '#A0B9D1', '#8BA8C8'],
    thinking: ['#C8A2FF', '#9D7BDB', '#8B6BC2'],
    talking: ['#A2FFB8', '#7BDB9D', '#6BC28B'],
  })

  useEffect(() => {
    const loadColors = async () => {
      try {
        const colors = await invoke<OrbColors>('get_orb_colors')
        setThemeColors(colors)
      } catch (err) {
        console.error('Failed to load orb colors:', err)
      }
    }

    loadColors()

    const unlisten = listen('theme_changed', () => {
      loadColors()
    })

    return () => {
      unlisten.then(fn => fn())
    }
  }, [])

  // Change colors based on agent state
  let orbColors: [string, string]
  let ringColor: string
  
  if (agentState === 'thinking') {
    orbColors = [themeColors.thinking[0], themeColors.thinking[1]]
    ringColor = themeColors.thinking[0]
  } else if (agentState === 'talking') {
    orbColors = [themeColors.talking[0], themeColors.talking[1]]
    ringColor = themeColors.talking[0]
  } else {
    orbColors = [themeColors.listening[0], themeColors.listening[1]]
    ringColor = themeColors.listening[0]
  }

  return (
    <div className="flex flex-col justify-start items-center m-0" style={{ overflow: 'visible' }}>
      <div className="rounded-[18px] p-[30px_35px_10px_35px] flex flex-col items-center w-full box-border" data-tauri-drag-region style={{ overflow: 'visible' }}>
        <button
          type="button"
          onClick={() => { void onToggleAudio(); }}
          aria-pressed={audioEnabled}
          title={audioEnabled ? 'Click to disable microphone' : 'Click to enable microphone'}
          className={`relative h-32 w-32 rounded-full border border-[#3a3a3a] shadow-[0_0_12px_rgba(0,0,0,0.35)] transition-opacity duration-150 ease-out focus:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:ring-[#cadcfc] ${audioEnabled ? '' : 'opacity-60'}`}
          style={{ padding: '8px', overflow: 'visible' }}
        >
          <svg className="absolute -inset-2 w-[calc(100%+16px)] h-[calc(100%+16px)] -rotate-90" style={{ transition: 'stroke 300ms ease-out', overflow: 'visible' }}>
            {agentState === 'listening' && (
              <circle
                cx="72"
                cy="72"
                r={radius}
                fill="none"
                stroke={ringColor}
                strokeWidth="2"
                strokeDasharray={circumference}
                strokeDashoffset={dashOffset}
                style={{ transition: 'stroke-dashoffset 0.016s linear, stroke 300ms ease-out' }}
              />
            )}
          </svg>
          <div className="absolute inset-2 rounded-full p-1 shadow-[inset_0_2px_8px_rgba(0,0,0,0.5)]" style={{ background: '#2d2d2d' }}>
            <div className="h-full w-full rounded-full shadow-[inset_0_0_12px_rgba(0,0,0,0.3)]" style={{ background: '#1e1e1e', overflow: 'hidden' }}>
              <Orb
                colors={orbColors}
                seed={1000}
                agentState={agentState}
                volumeMode="manual"
                manualInput={inputVolume}
                manualOutput={outputVolume}
                className="h-full w-full"
              />
            </div>
          </div>
        </button>
      </div>
    </div>
  )
}
