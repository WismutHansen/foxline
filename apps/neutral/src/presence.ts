import * as THREE from "three";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";
import { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";
import type { VisualTheme, PhosphorMode } from "./preferences";
export type { VisualTheme, PhosphorMode } from "./preferences";
export function kittPattern(energy: number) {
  const half =
    energy < 0.025
      ? 0
      : Math.ceil(Math.log10(1 + Math.min(1, Math.max(0, energy)) * 9) * 10);
  return Array.from({ length: 60 }, (_, i) =>
    Math.abs((i % 20) - 9.5) <
    Math.max(0, half - (Math.floor(i / 20) === 1 ? 0 : 2))
      ? 1
      : 0,
  );
}
export function relayColumns(width: number, height: number) {
  return Math.max(
    45,
    Math.min(512, Math.ceil((3.5 * width) / Math.max(1, height) / 0.046)),
  );
}
const curves = `
#define TAU 6.28318530718
vec3 knot(float t,float clock){float r=.53+.18*cos(3.*t+sin(clock*.19)*.35);return vec3(r*cos(2.*t),r*sin(2.*t),.26*sin(3.*t));}
vec3 form(float t,float n,float clock){
 float drift=clock*.22;
 if(n<.5)return vec3(.75*sin(t+drift*.55),.52*sin(2.*t-drift*.9),.24*cos(3.*t+drift));
 if(n<1.5){float r=.45+.24*cos(5.*t);return vec3(r*cos(t),r*sin(t),.24*sin(3.*t));}
 if(n<2.5)return knot(t,clock);
 if(n<3.5)return vec3(.44*cos(2.*t),.44*sin(2.*t),.7*sin(t));
 if(n<4.5)return vec3(.46*cos(3.*t)+.24*cos(7.*t+drift*1.4),.46*sin(4.*t)+.24*sin(9.*t-drift*1.2),.24*cos(5.*t+drift));
 return vec3(.43*cos(t)+.25*cos(5.*t),.43*sin(t)-.25*sin(5.*t),.3*sin(2.*t));
}
vec3 morph(float t,float clock){
 // Six continuous families; one transition takes ~9 seconds at idle.
 float phase=mod(clock*.11,6.);float a=floor(phase);float f=smoothstep(0.,1.,fract(phase));
 return mix(form(t,a,clock),form(t,mod(a+1.,6.),clock),f);
}
vec3 medusa(float u,float T){
 float i=u*20000.;float y=u*181.8;
 float k=8.*cos(y<59.?mod(i,9.):i*4.);
 float e=y/7.-16.;
 float d=length(vec2(k,e));d=d*d/89.+1.1;
 float c=d/1.5-T/12.+mod(i,2.)*3.;
 float q=k/d*3.-e*sin(k)+k/d/3.*(9.-d*4.*sin(d*d-T+sin(e)/2.))+70.;
 return vec3(q*sin(c),-(q*cos(c)+e),0.)/150.;
}
vec3 rotateShape(vec3 p,float clock){float a=clock*.075+.35;mat2 r=mat2(cos(a),-sin(a),sin(a),cos(a));p.xz=r*p.xz;a=sin(clock*.13)*.35;r=mat2(cos(a),-sin(a),sin(a),cos(a));p.yz=r*p.yz;return p;}
`;
export const phosphorVertexShader = `uniform float uTime;uniform float uEnergy;uniform float uMode;attribute float aT;attribute float aAge;varying float vAlpha;${curves}
void main(){float clock=uTime-aAge*.22;float t=aT*TAU;vec3 p=uMode<.5?knot(t,clock):(uMode<1.5?morph(t,clock):medusa(aT,clock*3.1416));p*=1.+uEnergy*.14;p=rotateShape(p,clock);vAlpha=exp(-aAge*.5)*(.34+.18*sin(t*3.-clock*1.4));gl_Position=projectionMatrix*modelViewMatrix*vec4(p,1.);}`;
const lineFragment = `uniform vec3 uColor;varying float vAlpha;void main(){gl_FragColor=vec4(uColor*vec3(1.6,2.3,1.25),vAlpha);}`;
const cloudVertex = `uniform float uTime;uniform float uEnergy;uniform float uPixelRatio;uniform float uMode;attribute float aSeed;varying float vAlpha;${curves}
void main(){if(uMode>2.5){float t=aSeed*20000.;float y=t/110.;float k=8.*cos(y<59.?mod(t,9.):t*4.);float e=y/7.-16.;float d=length(vec2(k,e));d=d*d/89.+1.1;float c=d/1.5-uTime*3.1416/12.+mod(floor(t*.5),2.)*3.;float q=k/d*3.-e*sin(k)+k/d/3.*(9.-d*4.*sin(d*d-uTime*3.1416+sin(e)/2.))+70.;vec3 p=vec3(q*sin(c)*1.6,-(q*cos(c)+e)*1.6,0.)/150.;p=rotateShape(p,uTime);p*=1.+uEnergy*.14;vAlpha=.26+.14*sin(t*.37);gl_Position=projectionMatrix*modelViewMatrix*vec4(p,1.);gl_PointSize=uPixelRatio*(2.1+uEnergy);return;}
float t=aSeed*TAU;float z=position.z;float az=t*137.508;float r=sqrt(max(0.,1.-z*z));vec3 shell=vec3(cos(az)*r,sin(az)*r,z)*.68;
float x=position.x;float y=position.y;float c=uTime*.19;
for(int i=0;i<14;i++){float nx=sin((-1.4+.18*sin(c))*y)+(.95+.15*cos(c*.71))*cos(1.7*x);y=sin(1.7*x)+.72*cos((-1.4+.18*sin(c))*y);x=nx;}
vec3 cloud=vec3(x*.37,y*.37,z*.18+.14*sin(x*3.+c));float swell=.5+.5*sin(uTime*.13);vec3 p=mix(cloud,shell,swell*.75);p=rotateShape(p,uTime);p*=1.+uEnergy*.14;
vAlpha=.12+.18*(z*.5+.5);gl_Position=projectionMatrix*modelViewMatrix*vec4(p,1.);gl_PointSize=uPixelRatio*(1.3+uEnergy*.6);}`;
const cloudFragment = `uniform vec3 uColor;varying float vAlpha;void main(){float d=length(gl_PointCoord-.5)*2.;if(d>1.)discard;gl_FragColor=vec4(uColor*1.6,vAlpha*(1.-d*d));
#include <colorspace_fragment>
}`;
const cellVertex = `attribute float aLevel;varying float vLevel;varying vec2 vUv;void main(){vLevel=aLevel;vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*instanceMatrix*vec4(position,1.);}`;
const cellFragment = `uniform vec3 uColor;uniform float uGlow;varying float vLevel;varying vec2 vUv;void main(){if(uGlow>.5){float a=exp(-dot((vUv-.5)*vec2(3.,2.),(vUv-.5)*vec2(3.,2.))*3.)*vLevel*.28;gl_FragColor=vec4(uColor*2.,a);}else{gl_FragColor=vec4(uColor*(.045+vLevel*2.2),1.);}
#include <colorspace_fragment>
}`;
/** Bounded rendering only. It never opens or routes media. */
export class PresenceRenderer {
  readonly canvas: HTMLCanvasElement;
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera = new THREE.PerspectiveCamera(34, 1, 0.1, 30);
  private kitt = new THREE.Group();
  private relay = new THREE.Group();
  private phosphor = new THREE.Group();
  private lines: THREE.LineSegments;
  private cloud: THREE.Points;
  private glow: THREE.Points;
  private levelAttribute: THREE.InstancedBufferAttribute;
  private vanes?: THREE.InstancedMesh;
  private base: THREE.Mesh;
  private environment: THREE.WebGLRenderTarget;
  private frame = 0;
  private last = 0;
  private time = 40;
  private energy = 0;
  private theme: VisualTheme = "phosphor";
  private reduced = false;
  private lost = false;
  private disposed = false;
  private columns = 0;
  private observer: ResizeObserver;
  private dummy = new THREE.Object3D();
  private size = new THREE.Vector2();
  private uniforms: {
    uTime: { value: number };
    uEnergy: { value: number };
    uColor: { value: THREE.Color };
    uMode: { value: number };
    uPixelRatio: { value: number };
  };
  private lose = (event: Event) => {
    event.preventDefault();
    this.lost = true;
    this.onUnavailable(
      "Graphics paused. Reload to restore the visualization. Voice controls are still available.",
    );
  };
  constructor(
    private host: HTMLElement,
    private levels: () => { input: number; output: number },
    private onUnavailable: (message: string) => void,
  ) {
    try {
      this.renderer = new THREE.WebGLRenderer({
        alpha: true,
        antialias: true,
        powerPreference: "low-power",
      });
      this.renderer.debug.onShaderError = () => {
        this.lost = true;
        this.onUnavailable(
          "Visualization unavailable. Voice and transcript controls still work.",
        );
      };
      this.canvas = this.renderer.domElement;
      this.canvas.setAttribute("aria-hidden", "true");
      host.append(this.canvas);
      this.renderer.setPixelRatio(Math.min(devicePixelRatio, 1.5));
      this.renderer.setClearColor(0, 0);
      this.camera.position.set(0, 0, 4.4);
      this.scene.add(
        this.kitt,
        this.phosphor,
        this.relay,
        new THREE.AmbientLight(0xb4cacc, 0.8),
      );
      const key = new THREE.DirectionalLight(0xe8efdf, 4.5);
      key.position.set(-2, 4, 3);
      this.scene.add(key);
      const generator = new THREE.PMREMGenerator(this.renderer);
      const room = new RoomEnvironment();
      this.environment = generator.fromScene(room, 0.04);
      this.scene.environment = this.environment.texture;
      room.dispose();
      generator.dispose();
      const css = getComputedStyle(document.documentElement);
      const color = (name: string, fallback: string) =>
        new THREE.Color(css.getPropertyValue(name).trim() || fallback);
      const signal = color(
        "--visual-phosphor",
        "#36ee89",
      ).convertLinearToSRGB();
      this.uniforms = {
        uTime: { value: 0 },
        uEnergy: { value: 0 },
        uColor: { value: signal },
        uMode: { value: 1 },
        uPixelRatio: { value: this.renderer.getPixelRatio() },
      };
      const count = 9 * 900 * 2,
        t = new Float32Array(count),
        age = new Float32Array(count);
      for (let layer = 0; layer < 9; layer++)
        for (let i = 0; i < 900; i++)
          for (let end = 0; end < 2; end++) {
            const n = layer * 1800 + i * 2 + end;
            t[n] = (i + end) / 900;
            age[n] = layer;
          }
      const trace = new THREE.BufferGeometry();
      trace.setAttribute(
        "position",
        new THREE.BufferAttribute(new Float32Array(count * 3), 3),
      );
      trace.setAttribute("aT", new THREE.BufferAttribute(t, 1));
      trace.setAttribute("aAge", new THREE.BufferAttribute(age, 1));
      this.lines = new THREE.LineSegments(
        trace,
        new THREE.ShaderMaterial({
          uniforms: this.uniforms,
          vertexShader: phosphorVertexShader,
          fragmentShader: lineFragment,
          toneMapped: false,
          transparent: true,
          depthWrite: false,
          blending: THREE.AdditiveBlending,
        }),
      );
      this.lines.frustumCulled = false;
      this.glow = new THREE.Points(
        trace,
        new THREE.ShaderMaterial({
          uniforms: this.uniforms,
          vertexShader: phosphorVertexShader.replace(
            "gl_Position=",
            "gl_PointSize=9.;gl_Position=",
          ),
          fragmentShader:
            "uniform vec3 uColor;varying float vAlpha;void main(){vec2 p=gl_PointCoord-.5;gl_FragColor=vec4(uColor*vec3(.8,1.3,.8),exp(-dot(p,p)*18.)*vAlpha*.045);}",
          transparent: true,
          depthWrite: false,
          blending: THREE.AdditiveBlending,
          toneMapped: false,
        }),
      );
      this.glow.frustumCulled = false;
      const grid = new THREE.Mesh(
        new THREE.PlaneGeometry(2.3, 1.55),
        new THREE.ShaderMaterial({
          uniforms: this.uniforms,
          vertexShader:
            "varying vec2 vUv;void main(){vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}",
          fragmentShader:
            "uniform vec3 uColor;varying vec2 vUv;void main(){vec2 g=abs(fract(vUv*vec2(10.,6.)+.5)-.5);float grid=(1.-smoothstep(.007,.018,min(g.x,g.y)))*.13;float axis=(1.-smoothstep(.001,.003,abs(vUv.x-.5)))+(1.-smoothstep(.001,.003,abs(vUv.y-.5)));float mask=1.-smoothstep(.24,.51,length((vUv-.5)*vec2(.85,1.2)));gl_FragColor=vec4(uColor*.45,(grid+axis*.09)*mask);}",
          transparent: true,
          depthWrite: false,
          blending: THREE.AdditiveBlending,
          toneMapped: false,
        }),
      );
      grid.position.z = -0.6;
      this.phosphor.add(grid);
      const points = 16000,
        p = new Float32Array(points * 3),
        seeds = new Float32Array(points);
      for (let i = 0; i < points; i++) {
        seeds[i] = i / points;
        p[i * 3] = Math.sin(i * 127.1) * 0.7;
        p[i * 3 + 1] = Math.sin(i * 311.7) * 0.7;
        p[i * 3 + 2] = 1 - (2 * (i + 0.5)) / points;
      }
      const cloudGeometry = new THREE.BufferGeometry();
      cloudGeometry.setAttribute("position", new THREE.BufferAttribute(p, 3));
      cloudGeometry.setAttribute("aSeed", new THREE.BufferAttribute(seeds, 1));
      this.cloud = new THREE.Points(
        cloudGeometry,
        new THREE.ShaderMaterial({
          uniforms: this.uniforms,
          vertexShader: cloudVertex,
          fragmentShader: cloudFragment,
          transparent: true,
          depthWrite: false,
          blending: THREE.AdditiveBlending,
        }),
      );
      this.cloud.frustumCulled = false;
      this.lines.scale.setScalar(1.2);
      this.glow.scale.copy(this.lines.scale);
      this.cloud.scale.setScalar(1.12);
      this.phosphor.add(this.lines, this.cloud, this.glow);
      this.levelAttribute = new THREE.InstancedBufferAttribute(
        new Float32Array(60),
        1,
      );
      for (const glow of [false, true]) {
        const geometry = new THREE.PlaneGeometry(
          glow ? 0.19 : 0.13,
          glow ? 0.062 : 0.027,
        );
        geometry.setAttribute("aLevel", this.levelAttribute);
        const material = new THREE.ShaderMaterial({
          uniforms: {
            uColor: { value: color("--visual-kitt", "#ef5c65") },
            uGlow: { value: glow ? 1 : 0 },
          },
          vertexShader: cellVertex,
          fragmentShader: cellFragment,
          transparent: glow,
          depthWrite: !glow,
          blending: glow ? THREE.AdditiveBlending : THREE.NormalBlending,
        });
        const mesh = new THREE.InstancedMesh(geometry, material, 60);
        for (let i = 0; i < 60; i++) {
          this.dummy.position.set(
            (Math.floor(i / 20) - 1) * 0.195,
            ((i % 20) - 9.5) * 0.037,
            glow ? 0.005 : 0,
          );
          this.dummy.updateMatrix();
          mesh.setMatrixAt(i, this.dummy.matrix);
        }
        this.kitt.add(mesh);
      }
      this.kitt.scale.setScalar(1.4);
      this.base = new THREE.Mesh(
        new RoundedBoxGeometry(2.19, 0.048, 1.48, 3, 0.015),
        new THREE.MeshStandardMaterial({
          color: 0x161f20,
          roughness: 0.34,
          metalness: 0.7,
          envMapIntensity: 0.35,
        }),
      );
      this.base.position.y = -0.17;
      this.relay.add(this.base);
      this.relay.rotation.set(0.63, 0, 0);
      this.observer = new ResizeObserver(() => this.resize());
      this.observer.observe(host);
      this.canvas.addEventListener("webglcontextlost", this.lose);
      this.setTheme("phosphor", "metamorph");
      this.resize();
      this.frame = requestAnimationFrame(this.tick);
    } catch (error) {
      this.dispose();
      throw error;
    }
  }
  setTheme(theme: VisualTheme, mode: PhosphorMode) {
    this.theme = theme;
    this.kitt.visible = theme === "kitt";
    this.relay.visible = theme === "relay";
    this.phosphor.visible = theme === "phosphor";
    this.cloud.visible = mode === "cloud" || mode === "medusa";
    this.lines.visible = this.glow.visible = mode !== "cloud" && mode !== "medusa";
    this.uniforms.uMode.value = mode === "torus" ? 0 : mode === "metamorph" ? 1 : mode === "medusa" ? 2 : 3;
    this.resize();
  }
  setReduced(value: boolean) {
    this.reduced = value;
    this.last = 0;
  }
  private drawable = true;
  private resize() {
    const { width, height } = this.host.getBoundingClientRect();
    this.drawable = width > 0 && height > 0;
    if (!this.drawable) {
      this.last = 0;
      return;
    }
    this.size.set(width, height);
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    if (this.theme === "relay") this.fitRelay(relayColumns(width, height));
  }
  private fitRelay(columns: number) {
    if (columns === this.columns) return;
    this.columns = columns;
    if (this.vanes) {
      this.relay.remove(this.vanes);
      this.vanes.geometry.dispose();
      (this.vanes.material as THREE.Material).dispose();
    }
    const geometry = new THREE.BoxGeometry(0.034, 1, 0.035);
    geometry.translate(0, 0.5, 0);
    const material = new THREE.MeshPhysicalMaterial({
      color: 0x7caaa2,
      metalness: 0.83,
      roughness: 0.26,
      clearcoat: 0.3,
    });
    material.onBeforeCompile = (shader) => {
      shader.uniforms.uTime = this.uniforms.uTime;
      shader.uniforms.uEnergy = this.uniforms.uEnergy;
      shader.vertexShader =
        "uniform float uTime,uEnergy;\n" + shader.vertexShader;
      shader.vertexShader = shader.vertexShader.replace(
        "#include <begin_vertex>",
        `#include <begin_vertex>
    vec2 cell=vec2(instanceMatrix[3].x,instanceMatrix[3].z);float r=length(cell*vec2(.8,1.));float wave=.5+.5*sin(cell.x*6.+cell.y*4.-uTime*1.15);float ripple=.5+.5*cos(r*13.-uTime*2.8);float height=.025+wave*.045+(pow(ripple,2.)*.35+.055)*uEnergy;transformed.y*=height;`,
      );
    };
    this.vanes = new THREE.InstancedMesh(geometry, material, columns * 29);
    this.vanes.frustumCulled = false;
    const start = new THREE.Color(0x3d6f6b),
      end = new THREE.Color(0xd5cdb0),
      matrix = new THREE.Matrix4();
    for (let z = 0; z < 29; z++)
      for (let x = 0; x < columns; x++) {
        this.vanes.setMatrixAt(
          z * columns + x,
          matrix.makeTranslation(
            (x - (columns - 1) / 2) * 0.046,
            -0.132,
            (z - 14) * 0.046,
          ),
        );
        this.vanes.setColorAt(
          z * columns + x,
          start.clone().lerp(end, x / (columns - 1)),
        );
      }
    this.base.scale.x = (columns * 0.046 + 0.12) / 2.19;
    this.relay.add(this.vanes);
  }
  private tick = (now: number) => {
    if (this.disposed) return;
    this.frame = requestAnimationFrame(this.tick);
    if (document.hidden || this.lost || !this.drawable) {
      this.last = 0;
      return;
    }
    if (this.last && now - this.last < (this.reduced ? 120 : 32)) return;
    const dt = this.last ? Math.min((now - this.last) / 1000, 0.1) : 0;
    this.last = now;
    const levels = this.levels(),
      target = this.reduced
        ? 0
        : Math.min(1, Math.max(levels.input, levels.output));
    this.energy += (target - this.energy) * (1 - Math.exp(-dt * 14));
    if (!this.reduced) this.time += dt * (1 + this.energy * 1.6);
    this.uniforms.uTime.value = this.time;
    this.uniforms.uEnergy.value = this.reduced ? 0 : this.energy;
    if (this.theme === "kitt") {
      const pattern = kittPattern(this.reduced ? 0 : this.energy);
      for (let i = 0; i < 60; i++) this.levelAttribute.setX(i, pattern[i]);
      this.levelAttribute.needsUpdate = true;
    }
    this.renderer.render(this.scene, this.camera);
  };
  dispose() {
    this.disposed = true;
    cancelAnimationFrame(this.frame);
    this.observer?.disconnect();
    this.canvas?.removeEventListener("webglcontextlost", this.lose);
    const geometries = new Set<THREE.BufferGeometry>(),
      materials = new Set<THREE.Material>();
    this.scene.traverse((object) => {
      const mesh = object as THREE.Mesh;
      if (mesh.geometry) geometries.add(mesh.geometry);
      if (mesh.material)
        for (const material of Array.isArray(mesh.material)
          ? mesh.material
          : [mesh.material])
          materials.add(material);
    });
    for (const geometry of geometries) geometry.dispose();
    for (const material of materials) material.dispose();
    this.environment?.dispose();
    this.renderer?.dispose();
    this.renderer?.forceContextLoss();
    this.canvas?.remove();
  }
}
