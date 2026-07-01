import { useCallback, useEffect, useRef, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { KittVoice, KittState } from './components/KittVoice'
import { OrbUI } from './components/OrbUI'
import { AgentState } from './components/Orb'
import { getConfig, setLastAgent, setAudioEnabled, getAudioEnabled } from './config/app-config'
import { MicrophonePcmStreamer, StreamingAudioPlayer, VoiceGatewayClient, type VoicePhase } from '@foxline/voice-client'

interface MicrophoneDevice {
  device_id: string
  label: string
  is_default: boolean
}

const NUM_SEGMENTS = 12
const MIN_LIT_SEGMENTS = 0
const MAX_LIT_CENTER = NUM_SEGMENTS
const MAX_LIT_SIDES_RELATIVE = 4
const LEVEL_CHANGE_SPEED = 0.25
const LEVEL_CHANGE_SPEED_SPEAKING = 0.45

type UIMode = 'kitt' | 'orb'
type AppState = 'idle' | 'listening' | 'thinking' | 'speaking'

function phaseToAppState(phase: VoicePhase): AppState {
  if (phase === 'thinking') return 'thinking'
  if (phase === 'speaking') return 'speaking'
  if (phase === 'idle' || phase === 'error') return 'idle'
  return 'listening'
}

function App() {
  const [uiMode, setUiMode] = useState<UIMode>('orb')
  const uiModeRef = useRef<UIMode>('orb')
  const [currentLitSegments, setCurrentLitSegments] = useState([MIN_LIT_SEGMENTS, MIN_LIT_SEGMENTS, MIN_LIT_SEGMENTS])
  const [targetLitSegments, setTargetLitSegments] = useState([MIN_LIT_SEGMENTS, MIN_LIT_SEGMENTS, MIN_LIT_SEGMENTS])
  const [isDraggable, setIsDraggable] = useState(false)
  const [inputVolume, setInputVolume] = useState(0)
  const [outputVolume, setOutputVolume] = useState(0)
  const [micActivity, setMicActivity] = useState(0)
  const [appState, setAppState] = useState<AppState>('idle')
  const [audioEnabled, setAudioEnabledState] = useState(true)
  const [permissionError, setPermissionError] = useState<string | null>(null)
  const [isInitialized, setIsInitialized] = useState(false)
  const [vadProgress] = useState(0)

  const animationRef = useRef<number | null>(null)
  const gatewayClientRef = useRef<VoiceGatewayClient | null>(null)
  const gatewayPlayerRef = useRef<StreamingAudioPlayer | null>(null)
  const gatewayMicRef = useRef<MicrophonePcmStreamer | null>(null)
  const transcriptRef = useRef<string>('')
  const audioEnabledRef = useRef<boolean>(true)
  const appStateRef = useRef<AppState>('idle')
  const smoothInputVolumeRef = useRef<number>(0)
  const smoothOutputVolumeRef = useRef<number>(0)
  const smoothMicActivityRef = useRef<number>(0)

  useEffect(() => {
    uiModeRef.current = uiMode
    setLastAgent(uiMode).catch(console.error)
  }, [uiMode])

  useEffect(() => {
    appStateRef.current = appState
  }, [appState])

  const startGatewayMic = useCallback(async () => {
    if (gatewayMicRef.current || !gatewayClientRef.current) return
    gatewayMicRef.current = new MicrophonePcmStreamer()
    gatewayMicRef.current.onAudioFrame((data) => gatewayClientRef.current?.sendAudioPcm16(data.pcm16))
    await gatewayMicRef.current.start()
  }, [])

  const stopGatewayMic = useCallback(() => {
    gatewayMicRef.current?.stop()
    gatewayMicRef.current = null
  }, [])

  const applyAudioEnabled = useCallback(async (enabled: boolean, options?: { allowActivePlayback?: boolean }) => {
    const allowActivePlayback = options?.allowActivePlayback ?? false
    audioEnabledRef.current = enabled
    setAudioEnabledState(enabled)
    await setAudioEnabled(enabled)

    if (enabled) {
      if (appStateRef.current === 'idle') {
        await startGatewayMic()
        setAppState('listening')
      }
      return
    }

    stopGatewayMic()
    const speaking = appStateRef.current === 'speaking' || appStateRef.current === 'thinking'
    if (!allowActivePlayback || !speaking) {
      gatewayPlayerRef.current?.stop()
      setAppState('idle')
    }
  }, [startGatewayMic, stopGatewayMic])

  const toggleAudioEnabled = useCallback(async () => {
    await applyAudioEnabled(!audioEnabledRef.current, { allowActivePlayback: true })
  }, [applyAudioEnabled])

  useEffect(() => {
    let unlistenToggleAudio: (() => void) | undefined
    let unlistenShowKitt: (() => void) | undefined
    let unlistenShowOrb: (() => void) | undefined
    let unlistenShowLastAgent: (() => void) | undefined
    let unlistenListMicrophones: (() => void) | undefined

    const setupGlobalShortcuts = async () => {
      const currentWindow = getCurrentWindow()

      unlistenToggleAudio = await listen('toggle_audio_shortcut', async () => {
        await toggleAudioEnabled()
      })

      unlistenShowKitt = await listen('show_kitt_shortcut', async () => {
        const isVisible = await currentWindow.isVisible()
        if (!isVisible || uiModeRef.current !== 'kitt') {
          if (!isVisible) {
            await currentWindow.show()
            await currentWindow.setFocus()
          }
          gatewayPlayerRef.current?.stop()
          setUiMode('kitt')
          if (appStateRef.current === 'speaking' || appStateRef.current === 'thinking') setAppState('listening')
          if (audioEnabledRef.current && appStateRef.current === 'idle') {
            await startGatewayMic()
            setAppState('listening')
          }
        } else {
          await applyAudioEnabled(false)
          await currentWindow.hide()
        }
      })

      unlistenShowOrb = await listen('show_orb_shortcut', async () => {
        const isVisible = await currentWindow.isVisible()
        if (!isVisible || uiModeRef.current !== 'orb') {
          if (!isVisible) {
            await currentWindow.show()
            await currentWindow.setFocus()
          }
          gatewayPlayerRef.current?.stop()
          setUiMode('orb')
          if (appStateRef.current === 'speaking' || appStateRef.current === 'thinking') setAppState('listening')
          if (audioEnabledRef.current && appStateRef.current === 'idle') {
            await startGatewayMic()
            setAppState('listening')
          }
        } else {
          await applyAudioEnabled(false)
          await currentWindow.hide()
        }
      })

      unlistenShowLastAgent = await listen('show_last_agent_shortcut', async () => {
        const isVisible = await currentWindow.isVisible()
        if (!isVisible) {
          await currentWindow.show()
          await currentWindow.setFocus()
          if (audioEnabledRef.current && appStateRef.current === 'idle') {
            await startGatewayMic()
            setAppState('listening')
          }
        } else {
          await applyAudioEnabled(false)
          await currentWindow.hide()
        }
      })

      unlistenListMicrophones = await listen('list_microphones_request', async () => {
        try {
          const devices = await navigator.mediaDevices.enumerateDevices()
          const microphones: MicrophoneDevice[] = devices
            .filter((device) => device.kind === 'audioinput')
            .map((device, index) => ({
              device_id: device.deviceId,
              label: device.label || `Microphone ${index + 1}`,
              is_default: device.deviceId === 'default' || index === 0,
            }))
          await invoke('update_microphone_list', { microphones })
        } catch (error) {
          console.error('[Overlayz] Failed to list microphones:', error)
        }
      })
    }

    setupGlobalShortcuts().catch(console.error)

    return () => {
      unlistenToggleAudio?.()
      unlistenShowKitt?.()
      unlistenShowOrb?.()
      unlistenShowLastAgent?.()
      unlistenListMicrophones?.()
    }
  }, [applyAudioEnabled, startGatewayMic, toggleAudioEnabled])

  useEffect(() => {
    let mounted = true

    const initGateway = async () => {
      try {
        const config = await getConfig()
        const storedAudioEnabled = await getAudioEnabled()
        if (!mounted) return

        audioEnabledRef.current = storedAudioEnabled
        setAudioEnabledState(storedAudioEnabled)

        gatewayPlayerRef.current = new StreamingAudioPlayer()
        gatewayPlayerRef.current.onPlaying = () => setAppState('speaking')
        gatewayPlayerRef.current.onStopped = () => setAppState(audioEnabledRef.current ? 'listening' : 'idle')

        const agent = config.foxline.agent
        const persona = config.foxline.persona || agent
        const workspaceOverride = config.foxline.workspace || `agents/${agent}`
        gatewayClientRef.current = new VoiceGatewayClient({
          url: config.foxline.url,
          clientId: 'foxline-overlayz',
          agent,
          persona,
          workspaceOverride,
          loadout: config.foxline.loadout,
          inputSampleRatesHz: [24000],
          binaryAudioSampleRateHz: 24000,
        })
        gatewayClientRef.current.onEvent(async (event) => {
          if (event.type === 'ready') {
            setAppState('idle')
            setPermissionError(null)
          } else if (event.type === 'phase') {
            setAppState(phaseToAppState(event.phase))
          } else if (event.type === 'user_transcript') {
            if (event.final) console.log('[Overlayz] user:', event.text)
          } else if (event.type === 'assistant_delta') {
            transcriptRef.current += event.delta
          } else if (event.type === 'turn_completed') {
            console.log('[Overlayz] assistant:', transcriptRef.current)
            transcriptRef.current = ''
          } else if (event.type === 'audio_pcm') {
            await gatewayPlayerRef.current?.enqueuePcm16(event.chunk, event.sample_rate)
          } else if (event.type === 'audio_reset') {
            gatewayPlayerRef.current?.stop()
          } else if (event.type === 'error') {
            console.error('[Overlayz] error:', event.message)
            setPermissionError(event.message)
            setAppState('idle')
          }
        })
        gatewayClientRef.current.connect()
        setIsInitialized(true)
      } catch (error) {
        console.error('[Overlayz] Gateway initialization error:', error)
        if (error instanceof Error) setPermissionError(error.message)
      }
    }

    initGateway()

    return () => {
      mounted = false
      gatewayClientRef.current?.disconnect()
      gatewayClientRef.current = null
      gatewayMicRef.current?.stop()
      gatewayMicRef.current = null
      gatewayPlayerRef.current?.stop()
      gatewayPlayerRef.current = null
    }
  }, [])

  useEffect(() => {
    let keyboardConfig: { switch_ui: string; clear_history: string; toggle_draggable: string } | null = null

    getConfig().then((config) => {
      keyboardConfig = config.keyboard
    })

    const startAudioCapture = async () => {
      if (!audioEnabledRef.current) return
      if (!isInitialized || !gatewayClientRef.current) return
      if (appState !== 'idle') return
      try {
        gatewayPlayerRef.current?.unlock()
        await startGatewayMic()
        setAppState('listening')
        setPermissionError(null)
      } catch (error) {
        console.error('[Overlayz] Failed to start gateway audio:', error)
        if (error instanceof Error) setPermissionError(error.message)
      }
    }

    const handleKeyDown = async (e: KeyboardEvent) => {
      if (isInitialized && appState === 'idle') await startAudioCapture()
      if (keyboardConfig?.toggle_draggable && (e.metaKey || e.key === keyboardConfig.toggle_draggable)) {
        setIsDraggable(true)
        document.body.style.cursor = 'move'
      }
      if (keyboardConfig?.switch_ui && e.key === keyboardConfig.switch_ui) {
        e.preventDefault()
        gatewayPlayerRef.current?.stop()
        setUiMode(uiMode === 'kitt' ? 'orb' : 'kitt')
        if (appState === 'speaking' || appState === 'thinking') setAppState('listening')
      }
      if (keyboardConfig?.clear_history && e.key === keyboardConfig.clear_history) {
        e.preventDefault()
        transcriptRef.current = ''
      }
    }

    const handleKeyUp = (e: KeyboardEvent) => {
      if (keyboardConfig?.toggle_draggable && (!e.metaKey && e.key === keyboardConfig.toggle_draggable)) {
        setIsDraggable(false)
        document.body.style.cursor = 'default'
      }
    }

    const handleMouseDown = async () => {
      if (isInitialized && appState === 'idle') await startAudioCapture()
      if (isDraggable) await getCurrentWindow().startDragging()
    }

    document.addEventListener('keydown', handleKeyDown)
    document.addEventListener('keyup', handleKeyUp)
    document.addEventListener('mousedown', handleMouseDown)

    return () => {
      document.removeEventListener('keydown', handleKeyDown)
      document.removeEventListener('keyup', handleKeyUp)
      document.removeEventListener('mousedown', handleMouseDown)
    }
  }, [isDraggable, isInitialized, appState, uiMode, startGatewayMic])

  useEffect(() => {
    const draw = () => {
      animationRef.current = requestAnimationFrame(draw)

      const micVolumeSample = gatewayMicRef.current?.getInputVolume?.() ?? 0
      let amplitude = 0
      let targetInputVolume = 0
      let targetOutputVolume = 0

      if (appState === 'listening' && gatewayMicRef.current) {
        amplitude = micVolumeSample
        targetInputVolume = amplitude
      } else if (appState === 'speaking' || appState === 'thinking') {
        amplitude = gatewayPlayerRef.current?.getLevel?.() ?? 0
        targetOutputVolume = amplitude
      }

      const smoothingFactor = 0.15
      const micSmoothingFactor = 0.3
      smoothInputVolumeRef.current += (targetInputVolume - smoothInputVolumeRef.current) * smoothingFactor
      smoothOutputVolumeRef.current += (targetOutputVolume - smoothOutputVolumeRef.current) * smoothingFactor
      smoothMicActivityRef.current += (micVolumeSample - smoothMicActivityRef.current) * micSmoothingFactor

      if (Math.abs(smoothInputVolumeRef.current) < 0.001) smoothInputVolumeRef.current = 0
      if (Math.abs(smoothOutputVolumeRef.current) < 0.001) smoothOutputVolumeRef.current = 0
      if (Math.abs(smoothMicActivityRef.current) < 0.001) smoothMicActivityRef.current = 0

      setInputVolume(smoothInputVolumeRef.current)
      setOutputVolume(smoothOutputVolumeRef.current)
      setMicActivity(smoothMicActivityRef.current)

      let centerTarget = MIN_LIT_SEGMENTS + amplitude * (MAX_LIT_CENTER - MIN_LIT_SEGMENTS)
      centerTarget = centerTarget < 1 ? 0 : Math.round(centerTarget / 2) * 2
      centerTarget = Math.max(MIN_LIT_SEGMENTS, centerTarget)

      let sideTarget = Math.max(MIN_LIT_SEGMENTS, centerTarget - MAX_LIT_SIDES_RELATIVE)
      sideTarget = sideTarget < 1 ? 0 : Math.round(sideTarget / 2) * 2

      setTargetLitSegments([sideTarget, centerTarget, sideTarget])
    }

    draw()

    return () => {
      if (animationRef.current) cancelAnimationFrame(animationRef.current)
    }
  }, [appState])

  useEffect(() => {
    const updateVisualizer = () => {
      setCurrentLitSegments((prev) => {
        const next = [...prev]
        const speed = appState === 'speaking' ? LEVEL_CHANGE_SPEED_SPEAKING : LEVEL_CHANGE_SPEED
        for (let i = 0; i < 3; i += 1) {
          if (Math.abs(next[i] - targetLitSegments[i]) < 0.5) {
            next[i] = Math.round(targetLitSegments[i])
          } else {
            next[i] += (targetLitSegments[i] - next[i]) * speed
            next[i] = next[i] < 2 ? 0 : Math.round(next[i] / 2) * 2
          }
          next[i] = Math.max(MIN_LIT_SEGMENTS, next[i])
        }
        return next
      })
    }

    const interval = setInterval(updateVisualizer, 1000 / 60)
    return () => clearInterval(interval)
  }, [targetLitSegments, appState])

  const getAgentState = (): AgentState => {
    if (appState === 'idle') return null
    if (appState === 'listening') return 'listening'
    if (appState === 'thinking') return 'thinking'
    return 'talking'
  }

  const getKittState = (): KittState => {
    if (appState === 'idle') return 'idle'
    if (appState === 'listening') return 'listening'
    if (appState === 'thinking') return 'processing'
    return 'speaking'
  }

  return (
    <>
      {permissionError && (
        <div style={{
          position: 'fixed',
          bottom: '10px',
          left: '50%',
          transform: 'translateX(-50%)',
          background: 'rgba(255, 0, 0, 0.9)',
          color: 'white',
          padding: '10px 20px',
          borderRadius: '5px',
          fontSize: '12px',
          zIndex: 1000,
          maxWidth: '300px',
          textAlign: 'center'
        }}>
          {permissionError}
        </div>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: '2px' }}>
        {uiMode === 'kitt' && (
          <KittVoice
            currentLitSegments={currentLitSegments}
            state={getKittState()}
            vadProgress={vadProgress}
            micActivity={micActivity}
            audioEnabled={audioEnabled}
            onToggleAudio={toggleAudioEnabled}
          />
        )}
        {uiMode === 'orb' && (
          <OrbUI
            inputVolume={inputVolume}
            outputVolume={outputVolume}
            agentState={getAgentState()}
            vadProgress={vadProgress}
            audioEnabled={audioEnabled}
            onToggleAudio={toggleAudioEnabled}
          />
        )}
      </div>
    </>
  )
}

export default App
