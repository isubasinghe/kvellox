import KvellModel.Protocol

namespace KvellModel.Safety

/-! Cross-machine properties. Local cache, transport, and request properties
are defined beside their machines and included in `all` below. -/

def ownership (s : Protocol.State) : Bool :=
  (s.io.slots.all fun c => c.all fun x => (s.cache.get x.buffer).isSome) &&
  ((List.range s.cache.buffers.size).all fun b => (s.cache.get b).all fun p =>
    (s.io.loading b == !p.ready))

def dataCoherent (s : Protocol.State) : Bool :=
  s.cache.buffers.all fun x => x.all fun p =>
    !p.ready || (s.visible[p.id]? == some p.data &&
      (p.dirty || s.kernel.disk[p.id]? == some p.data))

def frozenImages (s : Protocol.State) : Bool :=
  s.io.slots.all fun c => c.all fun ctx =>
    ctx.kind != .write || ctx.stage == .prepared ||
      (s.cache.get ctx.buffer).any fun p =>
        p.data == ctx.image && p.generation == ctx.generation

def waitersOwned (s : Protocol.State) : Bool := s.worker.requests.all fun r =>
  (!Worker.State.attached r || s.cache.contains r.page) &&
  (r.phase != .readWait || (List.range s.cache.buffers.size).any fun b =>
    (s.cache.get b).any (·.id == r.page) && s.io.loading b) &&
  (r.phase != .writing || s.io.slots.any fun c => c.any fun ctx =>
    ctx.kind == .write && ctx.stage != .prepared &&
      (s.cache.get ctx.buffer).any (·.id == r.page))

def all (s : Protocol.State) : Bool :=
  s.cache.wellFormed && s.io.wellFormed && s.worker.wellFormed &&
  ownership s && dataCoherent s && frozenImages s && waitersOwned s

def quiescent (s : Protocol.State) : Bool :=
  s.worker.requests.all (fun r => [.fresh, .done, .failed].contains r.phase) &&
  s.io.slots.all Option.isNone && s.cache.buffers.all (fun p => p.all (! ·.dirty))

end KvellModel.Safety
