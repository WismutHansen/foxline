import { useEffect, useState } from "react";

const NUM_SEGMENTS = 12;
const CENTER_INDEX_LOW = Math.floor((NUM_SEGMENTS - 1) / 2);
const CENTER_INDEX_HIGH = Math.ceil((NUM_SEGMENTS - 1) / 2);

export type KittState = "idle" | "listening" | "speaking" | "processing";

type BarMode = "scanning" | "stopping" | "stopped" | "growing";

interface KittVoiceProps {
  currentLitSegments: number[];
  state: KittState;
  vadProgress: number;
  micActivity: number;
  audioEnabled: boolean;
  onToggleAudio: () => Promise<void>;
}

export function KittVoice({
  currentLitSegments,
  state,
  vadProgress,
  micActivity,
  audioEnabled,
  onToggleAudio,
}: KittVoiceProps) {
  const hasSpeechActivity = audioEnabled && micActivity > 0.05;
  const isCountdownActive = audioEnabled && vadProgress > 0.05;
  const [barMode, setBarMode] = useState<BarMode>("scanning");
  const [animationPlayState, setAnimationPlayState] = useState<
    "running" | "paused"
  >("running");
  const [growingBarVisible, setGrowingBarVisible] = useState(false);
  const [growingBarOpacity, setGrowingBarOpacity] = useState(0);

  useEffect(() => {
    const isListening = state === "listening";
    const isSpeaking = state === "speaking";
    const isProcessing = state === "processing";
    const shouldShowSwell = hasSpeechActivity || isCountdownActive;
    let newMode: BarMode = "scanning";
    let newAnimationState: "running" | "paused" = "running";
    if (
      audioEnabled &&
      (isListening || isSpeaking || isProcessing) &&
      shouldShowSwell
    ) {
      // Speech activity or countdown detected: show draining bar (starts full, drains to empty)
      newMode = "growing";
      newAnimationState = "paused";
    }

    console.log(
      '[KITT] State:',
      state,
      'vadProgress:',
      vadProgress.toFixed(4),
      'hasSpeechActivity:',
      hasSpeechActivity,
      'micActivity:',
      micActivity.toFixed(4),
      'countdownActive:',
      isCountdownActive,
      'audioEnabled:',
      audioEnabled,
      'newMode:',
      newMode,
      'currentBarMode:',
      barMode,
      'animState:',
      newAnimationState
    );

    // Handle growing bar visibility and opacity
    if (newMode === "growing") {
      if (!growingBarVisible) {
        // Starting to show bar
        setGrowingBarVisible(true);
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            setGrowingBarOpacity(1);
          });
        });
      } else {
        // Continue showing bar: keep visible
        setGrowingBarOpacity(1);
      }
    } else if (newMode === "scanning") {
      // Scanning mode: hide the bar immediately
      if (growingBarVisible) {
        setGrowingBarOpacity(0);
        setGrowingBarVisible(false);
      }
    }

    // Force update if mode changes
    if (newMode !== barMode) {
      console.log('[KITT] Mode changing from', barMode, 'to', newMode);
    }
    
    setBarMode(newMode);
    setAnimationPlayState(newAnimationState);
  }, [state, vadProgress, growingBarVisible, barMode, hasSpeechActivity, isCountdownActive, audioEnabled, micActivity]);

  const renderColumn = (colIndex: number) => {
    const totalLit = Math.round(currentLitSegments[colIndex]);

    if (totalLit === 0) {
      return (
        <div className="flex flex-col justify-center items-center h-full w-[22px] gap-[2px]">
          {Array.from({ length: NUM_SEGMENTS }).map((_, segIndex) => (
            <div
              key={segIndex}
              className="h-[7px] w-full rounded-[1px] flex-shrink-0 transition-all duration-100 ease-in-out bg-[#330000] border-[#1a0000] opacity-70 border"
            />
          ))}
        </div>
      );
    }

    const segmentsToLightEachSide = Math.max(0, totalLit / 2 - 1);
    const startIndex = CENTER_INDEX_LOW - segmentsToLightEachSide;
    const endIndex = CENTER_INDEX_HIGH + segmentsToLightEachSide;

    return (
      <div className="flex flex-col justify-center items-center h-full w-[22px] gap-[2px]">
        {Array.from({ length: NUM_SEGMENTS }).map((_, segIndex) => {
          const isActive = segIndex >= startIndex && segIndex <= endIndex;
          return (
            <div
              key={segIndex}
              className={`h-[7px] w-full rounded-[1px] flex-shrink-0 transition-all duration-100 ease-in-out ${
                isActive
                  ? "bg-[#ff0000] border-[#ff0000] opacity-100 shadow-[0_0_5px_rgba(255,0,0,0.7),0_0_10px_rgba(255,0,0,0.7)]"
                  : "bg-[#330000] border-[#1a0000] opacity-70"
              } border`}
            />
          );
        })}
      </div>
    );
  };

  // Bar starts at 100% and drains to 0% as vadProgress goes from 0 to 1
  const barWidth = Math.max(0, (1 - vadProgress) * 100);
  const isBarNearEmpty = barWidth < 5; // Less than 5% width - hide earlier to prevent residual glow
  const handleBadgeClick = () => {
    void onToggleAudio();
  };

  return (
    <div
      className="flex flex-col justify-start items-center m-0"
      style={{ fontFamily: "'Orbitron', sans-serif" }}
    >
      <style>{`
        @keyframes kitt-swoosh {
          0% {
            left: 0%;
          }
          50% {
            left: 80%;
          }
          100% {
            left: 0%;
          }
        }
        @keyframes kitt-ghost-1 {
          0%, 100% {
            left: 1.5%;
            opacity: 0.6;
          }
          50% {
            left: 78.5%;
            opacity: 0.6;
          }
        }
        @keyframes kitt-ghost-2 {
          0%, 100% {
            left: 3%;
            opacity: 0.4;
          }
          50% {
            left: 77%;
            opacity: 0.4;
          }
        }
        @keyframes kitt-ghost-3 {
          0%, 100% {
            left: 4.5%;
            opacity: 0.25;
          }
          50% {
            left: 75.5%;
            opacity: 0.25;
          }
        }
        @keyframes kitt-ghost-4 {
          0%, 100% {
            left: 6%;
            opacity: 0.15;
          }
          50% {
            left: 74%;
            opacity: 0.15;
          }
        }
        @keyframes kitt-ghost-5 {
          0%, 100% {
            left: 8%;
            opacity: 0.08;
          }
          50% {
            left: 72%;
            opacity: 0.08;
          }
        }
        .kitt-swoosh-bar {
          animation: kitt-swoosh 1.2s cubic-bezier(0.45, 0.05, 0.55, 0.95) infinite;
          width: 20%;
          z-index: 6;
          transition: left 0.3s ease-out, width 0.3s ease-out;
        }
        .kitt-ghost-1 {
          animation: kitt-ghost-1 1.2s cubic-bezier(0.45, 0.05, 0.55, 0.95) infinite;
          width: 20%;
          z-index: 5;
          transition: left 0.3s ease-out, width 0.3s ease-out, opacity 0.3s ease-out;
        }
        .kitt-ghost-2 {
          animation: kitt-ghost-2 1.2s cubic-bezier(0.45, 0.05, 0.55, 0.95) infinite;
          width: 20%;
          z-index: 4;
          transition: left 0.3s ease-out, width 0.3s ease-out, opacity 0.3s ease-out;
        }
        .kitt-ghost-3 {
          animation: kitt-ghost-3 1.2s cubic-bezier(0.45, 0.05, 0.55, 0.95) infinite;
          width: 20%;
          z-index: 3;
          transition: left 0.3s ease-out, width 0.3s ease-out, opacity 0.3s ease-out;
        }
        .kitt-ghost-4 {
          animation: kitt-ghost-4 1.2s cubic-bezier(0.45, 0.05, 0.55, 0.95) infinite;
          width: 20%;
          z-index: 2;
          transition: left 0.3s ease-out, width 0.3s ease-out, opacity 0.3s ease-out;
        }
        .kitt-ghost-5 {
          animation: kitt-ghost-5 1.2s cubic-bezier(0.45, 0.05, 0.55, 0.95) infinite;
          width: 20%;
          z-index: 1;
          transition: left 0.3s ease-out, width 0.3s ease-out, opacity 0.3s ease-out;
        }
        .kitt-scanner-stopped .kitt-swoosh-bar,
        .kitt-scanner-stopped .kitt-ghost-1,
        .kitt-scanner-stopped .kitt-ghost-2,
        .kitt-scanner-stopped .kitt-ghost-3,
        .kitt-scanner-stopped .kitt-ghost-4,
        .kitt-scanner-stopped .kitt-ghost-5 {
          left: 40% !important;
          width: 5% !important;
        }
      `}</style>
      <div
        className="rounded-[18px] p-[30px_10px_10px_10px] flex flex-col items-center w-full box-border"
        data-tauri-drag-region
      >
        <div
          className="flex items-center justify-center bg-black p-[10px_15px] border border-[#222] rounded shadow-[inset_0_0_8px_rgba(0,0,0,0.8)] gap-[7px] mb-[10px] h-[120px] overflow-hidden"
          data-tauri-drag-region
        >
          {renderColumn(0)}
          {renderColumn(1)}
          {renderColumn(2)}
        </div>
        <div
          className="h-[5px] bg-[#330000] rounded-full overflow-hidden relative"
          style={{ width: "95px", filter: "contrast(1.2) brightness(1.1)" }}
        >
          {barMode === "scanning" && (
            <div style={{ position: "absolute", inset: 0 }}>
              <div
                className="kitt-ghost-5 h-full rounded-full absolute top-0"
                style={{
                  background:
                    "linear-gradient(to right, rgba(255,0,0,0), rgba(255,0,0,0.4), rgba(255,0,0,0))",
                  boxShadow: "0 0 3px rgba(255,0,0,0.2)",
                  filter: "blur(2px)",
                  animationPlayState: animationPlayState,
                }}
              />
              <div
                className="kitt-ghost-4 h-full rounded-full absolute top-0"
                style={{
                  background:
                    "linear-gradient(to right, rgba(255,0,0,0), rgba(255,0,0,0.5), rgba(255,0,0,0))",
                  boxShadow: "0 0 4px rgba(255,0,0,0.3)",
                  filter: "blur(1.8px)",
                  animationPlayState: animationPlayState,
                }}
              />
              <div
                className="kitt-ghost-3 h-full rounded-full absolute top-0"
                style={{
                  background:
                    "linear-gradient(to right, rgba(255,0,0,0), rgba(255,0,0,0.6), rgba(255,0,0,0))",
                  boxShadow: "0 0 5px rgba(255,0,0,0.4)",
                  filter: "blur(1.5px)",
                  animationPlayState: animationPlayState,
                }}
              />
              <div
                className="kitt-ghost-2 h-full rounded-full absolute top-0"
                style={{
                  background:
                    "linear-gradient(to right, rgba(255,0,0,0), rgba(255,0,0,0.7), rgba(255,0,0,0))",
                  boxShadow:
                    "0 0 6px rgba(255,0,0,0.5), 0 0 12px rgba(255,0,0,0.25)",
                  filter: "blur(1.2px)",
                  animationPlayState: animationPlayState,
                }}
              />
              <div
                className="kitt-ghost-1 h-full rounded-full absolute top-0"
                style={{
                  background:
                    "linear-gradient(to right, rgba(255,0,0,0), rgba(255,0,0,0.85), rgba(255,0,0,0))",
                  boxShadow:
                    "0 0 7px rgba(255,0,0,0.7), 0 0 14px rgba(255,0,0,0.4)",
                  filter: "blur(0.9px)",
                  animationPlayState: animationPlayState,
                }}
              />
              <div
                className="kitt-swoosh-bar h-full bg-[#ff0000] rounded-full absolute top-0"
                style={{
                  boxShadow:
                    "0 0 10px rgba(255,0,0,1), 0 0 20px rgba(255,0,0,0.8), 0 0 30px rgba(255,0,0,0.5), 0 0 40px rgba(255,0,0,0.3)",
                  filter: "blur(0.5px)",
                  animationPlayState: animationPlayState,
                }}
              />
            </div>
          )}

          {/* Speech detection bar - starts full and drains */}
          {growingBarVisible && !isBarNearEmpty && (
            <div
              className="h-full bg-[#ff0000] rounded-full absolute top-0"
              style={{
                width: `${barWidth}%`,
                left: `${(100 - barWidth) / 2}%`,
                boxShadow: "0 0 6px rgba(255,0,0,0.9), 0 0 12px rgba(255,0,0,0.6), 0 0 18px rgba(255,0,0,0.3)",
                filter: "blur(0.3px)",
                opacity: growingBarOpacity,
                transition:
                  barMode === "growing"
                    ? "width 16ms linear, left 16ms linear, opacity 300ms ease-out"
                    : "opacity 300ms ease-out",
              }}
            />
          )}
        </div>
      </div>
      <button
        type="button"
        onClick={handleBadgeClick}
        aria-pressed={audioEnabled}
        title={audioEnabled ? "Click to disable microphone" : "Click to enable microphone"}
        className={`bg-[#d4b404] text-black text-[0.6em] font-bold py-[3px] px-2 rounded-[3px] text-center tracking-[0.5px] border border-[#444] shadow-[0_1px_2px_rgba(0,0,0,0.4)] w-[95px] uppercase transition-opacity duration-150 ease-out cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-[#ff0000]/50 ${audioEnabled ? '' : 'opacity-60'}`}
      >
        {audioEnabled ? (
          <>
            {state === "idle" && "NORMAL CRUISE"}
            {state === "listening" && "LISTENING"}
            {state === "speaking" && "SPEAKING"}
            {state === "processing" && "PROCESSING"}
          </>
        ) : (
          "MIC OFF"
        )}
      </button>
    </div>
  );
}
