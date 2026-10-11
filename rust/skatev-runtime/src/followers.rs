//! Independent Skate sessions on the player's worker, never another thread
//! publishing into the engine's input bus. No player trick/input replay.
use crate::{coords, world, worker::{CharacterRequest, Log, Presenter}};
use bevy_math::{Mat4, Vec3};
use skate_host::bridge::{InputFrame, Session, Pose, PreparedCollision};
use std::{collections::{VecDeque, HashMap}, path::Path, sync::{Arc, mpsc}};

// Immutable navigation data, prepared alongside collision off the skating
// worker. Large triangles use a bounded fallback rather than filling millions
// of grid cells; local queries still include them conservatively.
struct NavTriangle { vertices: [Vec3;3], normal: Vec3 }
struct Navigation {
    triangles: Vec<NavTriangle>, cells: HashMap<(i32,i32),Vec<usize>>, wide: Vec<usize>,
}
impl Navigation {
    const CELL: f32 = 8.;
    fn cell(p: Vec3) -> (i32,i32) {
        ((p.x/Self::CELL).floor() as i32,(p.y/Self::CELL).floor() as i32)
    }
    fn new(area: &world::Area) -> Self {
        let mut nav=Self{triangles:Vec::with_capacity(area.triangles.len()),cells:HashMap::new(),wide:Vec::new()};
        for triangle in &area.triangles {
            let vertices=triangle.map(Vec3::from_array).map(coords::from_skate);
            let [a,b,c]=vertices;
            if !a.is_finite()||!b.is_finite()||!c.is_finite(){continue;}
            let index=nav.triangles.len();
            nav.triangles.push(NavTriangle{vertices,normal:(b-a).cross(c-a).normalize_or_zero()});
            let lo=Self::cell(a.min(b).min(c));let hi=Self::cell(a.max(b).max(c));
            let width=hi.0 as i64-lo.0 as i64+1;let height=hi.1 as i64-lo.1 as i64+1;
            if width>256||height>256||width*height>256 {
                nav.wide.push(index);continue;
            }
            for x in lo.0..=hi.0 {for y in lo.1..=hi.1 {nav.cells.entry((x,y)).or_default().push(index);}}
        }
        nav
    }
    fn nearby(&self, point: Vec3, radius: f32) -> impl Iterator<Item=&NavTriangle> {
        let lo=Self::cell(point-Vec3::splat(radius));let hi=Self::cell(point+Vec3::splat(radius));
        (lo.0..=hi.0).flat_map(move |x|(lo.1..=hi.1).flat_map(move |y|
            self.cells.get(&(x,y)).into_iter().flatten().copied()))
            .chain(self.wide.iter().copied()).map(|i|&self.triangles[i])
    }
}
type PendingCollision = mpsc::Receiver<Result<(Arc<world::Area>, Navigation, PreparedCollision),String>>;

pub const MAX: usize = 4;
#[derive(Clone)]
pub struct Request { pub id: u32, pub entity: u32, pub spawn: Vec3, pub heading: f32, pub character: CharacterRequest }
#[derive(Clone, Copy)]
pub struct Goal { pub id: u32, pub at: Vec3, pub current: Vec3, pub speed: f32, pub tricks: bool }
#[derive(Clone)]
pub struct Snapshot {
    pub id: u32, pub tick: u64, pub root: Vec3, pub heading: f32, pub velocity: Vec3,
    pub bones: Arc<Vec<Mat4>>, pub board: [Mat4; 7], pub board_entity: Mat4,
}
#[derive(Clone, Copy, Debug)]
struct Line { entry: Vec3, exit: Vec3, direction: Vec3 }

fn horizontal(v: Vec3) -> Vec3 { Vec3::new(v.x, v.y, 0.) }
fn distance(a: Vec3, b: Vec3) -> f32 { horizontal(a-b).length() }

// A route lookahead is a steering point, not a stop sign. Only apply arrival
// braking when the host requests a stop at the end of the route.
fn cruising_speed(leader_speed: f32, gap: f32, angle: f32) -> f32 {
    let moving=leader_speed>0.5;
    let wanted=if moving { (leader_speed+(gap-5.).max(0.)*0.5).clamp(0.,11.) }
        else { (2.0*1.5*(gap-1.0).max(0.)).sqrt().min(8.) };
    // Ordinary bends and obstacle detours should not force walking speed.
    let turn=(1.-((angle.abs()-0.6).max(0.)/2.5)).clamp(0.35,1.);
    wanted*turn
}

fn steering_pad(angle: f32) -> [i16;2] {
    if angle.abs()<0.025 {return [0,0];}
    // Compensate the Xbox stick deadzone. Diagonals engage Skate's hard-turn
    // intention; a pure sideways stick never enters that angular window.
    if angle.abs()>0.8 {return [(-angle.signum()*0.8*32767.) as i16,-19660];}
    let x=-angle.signum()*(0.25+0.75*(angle.abs()*1.4).min(1.));
    [(x*32767.) as i16,0]
}

#[derive(Clone,Copy,Debug)]
struct Blockage { low: bool, ahead: f32 }
fn blockage(nav: &Navigation, deck: Vec3, direction: Vec3) -> Option<Blockage> {
    let mut nearest:Option<Blockage>=None;
    for triangle in nav.nearby(deck,6.) {
        let [a,b,c]=triangle.vertices;
        let normal=triangle.normal;
        if normal.z.abs()>0.7 {continue;}
        let top=a.z.max(b.z).max(c.z)-deck.z;
        let bottom=a.z.min(b.z).min(c.z)-deck.z;
        if top<0.06||bottom>0.5 {continue;}
        // Intersect the route ray with each triangle edge in horizontal space,
        // retaining the actual height at the crossing (curbs/walls/fences).
        for (a,b) in [(a,b),(b,c),(c,a)] {
            let edge=horizontal(b-a);let delta=horizontal(a-deck);
            let denominator=direction.cross(edge).z;
            if denominator.abs()<1e-5 {continue;}
            let ahead=delta.cross(edge).z/denominator;
            let along=delta.cross(direction).z/denominator;
            if !(0.0..=1.0).contains(&along)||!(0.2..=6.0).contains(&ahead){continue;}
            if nearest.is_none_or(|n|ahead<n.ahead){nearest=Some(Blockage{low:top<0.65,ahead});}
        }
    }
    nearest
}

/// Choose a low, long, forward-facing rail along the follower's route.
/// All candidates are collision-derived; targets are not the player's tricks.
fn rail_line(area: &world::Area, at: Vec3, direction: Vec3) -> Option<Line> {
    let mut chosen = None;
    let mut cost = f32::INFINITY;
    for rail in &area.rails {
        for segment in rail.windows(2) {
            let mut a=coords::from_skate(Vec3::from_array(segment[0]));
            let mut b=coords::from_skate(Vec3::from_array(segment[1]));
            if horizontal(b-a).dot(direction) < 0. { std::mem::swap(&mut a, &mut b); }
            let length=distance(a,b);
            let tangent=horizontal(b-a).normalize_or_zero();
            let delta=horizontal(a-at);
            let ahead=delta.dot(direction);
            let side=(delta-direction*ahead).length();
            if length<3. || (b.z-a.z).abs()>0.5 || tangent.dot(direction)<0.85 ||
                !(3.0..=10.).contains(&ahead) || side>2.5 || !(0.12..=0.85).contains(&(a.z-at.z)) { continue; }
            let score=ahead+side*3.;
            if score<cost {cost=score; chosen=Some(Line {entry:a, exit:b, direction:tangent});}
        }
    }
    chosen
}

/// GTA-space floor under a point, near the expected elevation; excludes walls
/// and remote retained floors on another level of the map.
fn floor(nav: &Navigation, point: Vec3) -> Option<(f32, Vec3)> {
    let mut nearest=None;
    let mut error=3.0;
    for t in nav.nearby(point,0.) {
        let [a,b,c]=t.vertices;
        let normal=t.normal;
        if normal.z.abs()<0.35 {continue;}
        let denominator=(b.y-c.y)*(a.x-c.x)+(c.x-b.x)*(a.y-c.y);
        if denominator.abs()<1e-6 {continue;}
        let u=((b.y-c.y)*(point.x-c.x)+(c.x-b.x)*(point.y-c.y))/denominator;
        let v=((c.y-a.y)*(point.x-c.x)+(a.x-c.x)*(point.y-c.y))/denominator;
        if u<0. || v<0. || u+v>1. {continue;}
        let z=u*a.z+v*b.z+(1.-u-v)*c.z;
        let difference=(z-point.z).abs();
        if difference<error {error=difference;nearest=Some((z,if normal.z<0. {-normal} else {normal}));}
    }
    nearest
}

pub struct Agent {
    pub id: u32, pub entity: u32,
    session: Session, presenter: Presenter, area: Arc<world::Area>,
    pub goal: Goal,
    packet: u32, time: f32, cooldown: f32, jump: f32, flip: bool,
    line: Option<Line>, line_time: f32, recovering: f32,
    obstacles: Vec<(Vec3,f32)>,
    spawn: Vec3, heading: f32, ready: bool, braking: bool,
    route_areas: VecDeque<Arc<world::Area>>, blocked: f32,
    detour: Option<Vec3>, detour_time: f32, obstacle_cooldown: f32,
    navigation: Navigation, pending_collision: Option<PendingCollision>, last_pose: Option<Pose>,
    rail_scan: u32,
}
impl Agent {
    pub fn set_bodies<'a>(&mut self, bodies: impl Iterator<Item=&'a skate_host::bridge::HostBody>) {
        if self.packet%4!=0 {return;}
        let bodies:Vec<_>=bodies.filter(|b|(b.tag & 0x0fff_ffff)!=self.entity).cloned().collect();
        self.obstacles=bodies.iter().map(|b|(coords::from_skate(Vec3::from_array(b.position)),
            if b.collision_group==skate_host::bridge::VEHICLE_GROUP {3.0} else {1.2})).collect();
        self.session.set_host_bodies(bodies);
    }
    pub fn set_dynamic(&mut self, triangles: &[([[f32;3];3],u32,u32)]) {
        if self.packet%4==0 {
            let triangles:Vec<_>=triangles.iter().filter(|(_,_,tag)|(*tag & 0x0fff_ffff)!=self.entity).copied().collect();
            self.session.set_dynamic_surfaces(&triangles);
        }
    }
    pub fn new(root: &Path, area: Arc<world::Area>, request: Request, log: &Log) -> Result<Self,String> {
        let session=Session::new_tagged(root,area.triangles.clone(),area.tags.clone(),area.rails.clone(),
            coords::to_skate(request.spawn).to_array(),coords::skate_heading(request.heading))?;
        let mut presenter=Presenter::load(root,128,true,log).ok_or("follower presentation unavailable")?;
        presenter.set_character(Some(request.character),log);
        if !presenter.has_character() {return Err("follower skeleton could not be fitted".into());}
        let navigation=Navigation::new(&area);
        Ok(Self {id:request.id, entity:request.entity, session,presenter,area,
            goal:Goal{id:request.id,at:request.spawn,current:request.spawn,speed:0.,tricks:true},packet:0,time:0.,
            cooldown:2.+(request.id%7) as f32,jump:0.,flip:false,line:None,line_time:0.,recovering:0.,obstacles:Vec::new(),
            spawn:request.spawn,heading:request.heading,ready:false,braking:false,
            route_areas:VecDeque::new(),blocked:0.,detour:None,detour_time:0.,obstacle_cooldown:0.,
            navigation,pending_collision:None,last_pose:None,rail_scan:0})
    }
    pub fn step(&mut self, area: Option<&Arc<world::Area>>, log: &Log) -> Result<Option<Snapshot>,String> {
        if !self.ready {
            // The GTA follower kept moving while immutable assets loaded.
            // Hand over at its current feet rather than its old spawn point.
            if let Some((z,_))=floor(&self.navigation,self.goal.current) {
                self.spawn=Vec3::new(self.goal.current.x,self.goal.current.y,z);
            }
            self.session.set_hall_of_meat(false);
            self.session.set_bail_maximum_time(Some(f32::INFINITY));
            self.session.set_air_time_limit(Some(f32::INFINITY));
            self.session.set_difficulty(0);
            self.session.set_natural_stance(0); // regular stance: A is a normal push
            let toward=horizontal(self.goal.at-self.spawn);
            if toward.length()>1.5 {self.heading=coords::gta_heading(toward);}
            self.session.activate(coords::to_skate(self.spawn).to_array(),coords::skate_heading(self.heading))?;
            self.ready=true;
        }
        let p=self.last_pose.take().unwrap_or_else(||self.session.pose());
        let at=coords::from_skate(p.root.w_axis.truncate());
        if let Some(new)=area.filter(|a|!Arc::ptr_eq(a,&self.area)&&!self.route_areas.iter().any(|old|Arc::ptr_eq(old,a))) {
            self.route_areas.push_back(Arc::clone(new));
            while self.route_areas.len()>12 {self.route_areas.pop_front();}
        }
        // Keep collision under a lagging follower instead of replacing it with
        // the player's distant area. Saved overlapping route regions allow it
        // to catch up through areas GTA has already unloaded.
        let completed=self.pending_collision.as_ref().and_then(|pending|match pending.try_recv() {
            Ok(result)=>Some(result),
            Err(mpsc::TryRecvError::Empty)=>None,
            Err(mpsc::TryRecvError::Disconnected)=>Some(Err("collision preparation thread ended".into())),
        });
        if let Some(result)=completed {
            self.pending_collision=None;
            match result {
                Ok((new,nav,prepared)) if distance(at,new.center)<world::RADIUS-12.&&floor(&nav,at).is_some()=>{
                    self.session.install_collision(prepared)?;self.area=new;self.navigation=nav;
                }
                Ok(_)=>{},
                Err(error)=>log(&format!("AI collision {}: preparation failed: {error}",self.entity)),
            }
        }
        if self.pending_collision.is_none()&&self.packet%30==0&&distance(at,self.area.center)>world::RADIUS-48. {
            if let Some(new)=self.route_areas.iter().rev().find(|new|distance(at,new.center)<world::RADIUS-48.).cloned() {
                let builder=self.session.collision_builder();let (send,receive)=mpsc::channel();
                self.pending_collision=Some(receive);
                std::thread::spawn(move || {
                    crate::worker::set_thread_priority(-1);
                    let result=builder.build_tagged(new.triangles.clone(),new.tags.clone(),new.rails.clone())
                        .map(|prepared|{let nav=Navigation::new(&new);(new,nav,prepared)});
                    let _=send.send(result);
                });
            }
        }
        let dt=self.session.period();
        self.time+=dt; self.cooldown=(self.cooldown-dt).max(0.);
        self.obstacle_cooldown=(self.obstacle_cooldown-dt).max(0.);
        let at=coords::from_skate(p.root.w_axis.truncate());
        let (deck,rotation)=self.session.deck();
        let deck=coords::from_skate(Vec3::from_array(deck));
        let rotation=coords::rotation_from_skate(bevy_math::Quat::from_array(rotation));
        let velocity=coords::basis()*p.velocity;
        let mut forward=horizontal(rotation * -Vec3::Y).normalize_or_zero();
        if horizontal(velocity).length()>1. && forward.dot(velocity)<0. {forward=-forward;}
        if forward.length_squared()<0.5 {forward=horizontal(self.goal.at-at).normalize_or_zero();}
        // Do not run on a missing streaming region, or teleport mission actors.
        if distance(at,self.area.center)>world::RADIUS-12. {
            self.packet=self.packet.wrapping_add(1);
            if self.packet%120==0 {log(&format!("AI skater {}: waiting for route collision, no teleport",self.entity));}
            return self.snapshot(log);
        }
        if p.state.starts_with("Wipeout") || p.state.starts_with("Biped") {
            self.recovering+=dt;
            if self.recovering>4. {
                let Some((z,_))=floor(&self.navigation,at) else {
                    self.packet=self.packet.wrapping_add(1);return self.snapshot(log);
                };
                self.session.activate(coords::to_skate(Vec3::new(at.x,at.y,z)).to_array(),coords::skate_heading(coords::gta_heading(forward)))?;
                self.recovering=0.;self.line=None;self.jump=0.;self.cooldown=5.;
            }
        } else {self.recovering=0.;}
        let gap=distance(at,self.goal.at);
        if !self.goal.tricks || gap>20. {self.line=None;}
        if self.line.is_some() {
            self.line_time+=dt;
            if self.line_time>7. {self.line=None;self.cooldown=8.;}
        }
        if self.packet.wrapping_sub(self.rail_scan)>=30 && self.goal.tricks && self.cooldown==0. && self.line.is_none() && gap>3. && gap<16. && velocity.length()>3. {
            self.rail_scan=self.packet;
            self.line=rail_line(&self.area,deck,forward);
            if self.line.is_some() {self.line_time=0.; log(&format!("AI skater {}: chose nearby grind line",self.entity));}
        }
        let mut target=self.goal.at;
        let speed=horizontal(velocity).length();
        if gap>2. && speed<0.65 && p.state.starts_with("PhysicsGround") {self.blocked+=dt;} else {self.blocked=0.;}
        // Obstacle ollies are navigation, independent of optional show-off
        // tricks. A stopped skater must not keep pushing into a curb forever.
        let obstacle=if self.packet%8==0 {blockage(&self.navigation,deck,forward)} else {None};
        if self.jump==0. && self.obstacle_cooldown==0. &&
            (obstacle.is_some_and(|o|o.low&&o.ahead<(speed*0.32+1.5).max(2.5)&&speed>1.)||self.blocked>0.8) {
            self.jump=0.32;self.flip=false;self.obstacle_cooldown=2.5;
            log(&format!("AI skater {}: navigation ollie, blocked {:.2}s",self.entity,self.blocked));
        }
        if (self.blocked>3.||obstacle.is_some_and(|o|!o.low))&&self.detour.is_none() {
            let toward=horizontal(self.goal.at-at).normalize_or_zero();
            let side=Vec3::new(toward.y,-toward.x,0.);
            for offset in [side*3.,-side*3.,side*5.,-side*5.] {
                let candidate=deck+offset+toward*2.;
                if let Some((z,_))=floor(&self.navigation,candidate) && (z-deck.z).abs()<0.8 &&
                    blockage(&self.navigation,deck,offset.normalize_or_zero()).is_none() {
                    self.detour=Some(Vec3::new(candidate.x,candidate.y,z));self.detour_time=5.;
                    self.blocked=0.;log(&format!("AI skater {}: obstacle detour",self.entity));break;
                }
            }
        }
        if let Some(line)=self.line {
            target=if self.line_time<1.5 && distance(deck,line.entry)>2.5 {line.entry-line.direction*2.} else {line.exit+line.direction*3.};
            let before_entry=horizontal(line.entry-deck).dot(line.direction);
            if before_entry>0. && before_entry<2.5 && distance(deck,line.entry)<3. && self.jump==0. && self.cooldown==0. {
                self.jump=0.32;self.flip=false;self.cooldown=8.+(self.id%5) as f32;
            }
            if horizontal(deck-line.exit).dot(line.direction)>1. {self.line=None;}
        } else if self.goal.tricks && self.cooldown==0. && velocity.length()>4. && gap<16. {
            // A rising ramp or low ledge is an opportunity for an independent
            // ollie/flip; flat roads do not trigger random tricks.
            if let Some((z,n))=floor(&self.navigation,deck+forward*2.) {
                if z-deck.z>0.12 && z-deck.z<0.8 && n.z>0.45 {
                    self.jump=0.32;self.flip=self.id%2==0;self.cooldown=7.+(self.id%5) as f32;
                    log(&format!("AI skater {}: chose ramp/ledge trick",self.entity));
                }
            }
            if self.jump==0. {self.cooldown=0.5;}
        }
        let desired=horizontal(target-at);
        if let Some(detour)=self.detour {
            self.detour_time-=dt;
            if distance(at,detour)<1.5||self.detour_time<=0. {self.detour=None;}
        }
        let desired=self.detour.map_or(desired,|detour|horizontal(detour-at));
        let mut direction=desired.normalize_or_zero();
        for (center,radius) in &self.obstacles {
            let delta=horizontal(*center-at);
            let ahead=delta.dot(direction);
            if ahead>0. && ahead<6. && (delta-direction*ahead).length()<*radius+0.8 && (center.z-at.z).abs()<3. {
                let right=Vec3::new(direction.y,-direction.x,0.);
                let away=if delta.dot(right)>0. {-right} else {right};
                direction=(direction+away*1.5).normalize_or_zero();
                break;
            }
        }
        let angle=forward.cross(direction).z.atan2(forward.dot(direction));
        let steering=steering_pad(angle);
        let wanted=cruising_speed(self.goal.speed,gap,angle).min(if angle.abs()>1.4 {2.5} else {11.});
        // Coast around the requested speed rather than alternate push/brake
        // on every small waypoint or velocity update.
        self.braking=if self.braking {speed>wanted+0.25}
            else {speed>wanted+if wanted<1. {0.4} else {1.5}};
        // A taps push, B brakes. Steering always comes from this agent's route.
        let push=if self.session.natural_stance().1 {0x1000} else {0x4000};
        let buttons=if p.state.starts_with("Wipeout")||p.state.starts_with("Biped") {0}
            else if self.braking {0x2000}
            else if speed<wanted-0.5 && self.time%0.55<0.16 {push} else {0};
        if self.packet%120==0 {
            log(&format!("AI control {}: gap={gap:.2} requested={:.2} speed={speed:.2} wanted={wanted:.2} angle={angle:.2} brake={} state={}",self.entity,self.goal.speed,self.braking,p.state));
        }
        let mut right=[0,0];
        if self.jump>0. {
            self.jump=(self.jump-dt).max(0.);
            right=if self.jump>0.06 {[0,-32767]} else if self.flip {[26000,30000]} else {[0,32767]};
        }
        self.packet=self.packet.wrapping_add(1);
        self.session.collect(InputFrame::from_pad(buttons,[0,0],steering,right,self.packet),dt);
        self.session.advance()?;
        self.snapshot(log)
    }
    fn snapshot(&mut self,log:&Log) -> Result<Option<Snapshot>,String> {
        let p=self.session.pose();
        let (bones,board,board_entity)=self.presenter.follower_pose(&p,log).ok_or("follower pose unavailable")?;
        let (root,q)=coords::transform_from_skate(p.root);
        let snapshot=Snapshot {id:self.id,tick:self.packet as u64,root,heading:coords::gta_heading(q*(coords::basis()*Vec3::Z)),
            velocity:coords::basis()*p.velocity,bones,board,board_entity};
        self.last_pose=Some(p);
        Ok(Some(snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unindexed(nav: &Navigation) -> Navigation {
        Navigation {triangles:nav.triangles.iter().map(|t|NavTriangle{vertices:t.vertices,normal:t.normal}).collect(),
            cells:HashMap::new(),wide:(0..nav.triangles.len()).collect()}
    }
    #[test] fn indexed_queries_match_full_scan_across_cell_boundaries_and_large_triangles() {
        let mut area=world::build_area(Vec3::ZERO,[
            [[-300.,-300.,0.],[300.,-300.,0.],[-300.,300.,0.]],
            [[300.,-300.,0.],[300.,300.,0.],[-300.,300.,0.]],
        ].into_iter());
        for x in -20..20 {for y in -20..20 {
            let x=x as f32*2.;let y=y as f32*2.;
            let triangle=[Vec3::new(x,y,0.),Vec3::new(x+1.,y,0.),Vec3::new(x+1.,y,0.25)];
            area.triangles.push(triangle.map(coords::to_skate).map(|v|v.to_array()));
        }}
        let nav=Navigation::new(&area);let reference=unindexed(&nav);
        assert!(!nav.wide.is_empty());
        for x in [-16.01,-8.,-0.01,0.,7.99,8.,16.01] {for y in [-12.,0.1,8.,18.] {
            let point=Vec3::new(x,y,0.4);
            assert_eq!(floor(&nav,point).map(|v|v.0),floor(&reference,point).map(|v|v.0));
            for direction in [Vec3::X,Vec3::Y,-Vec3::X,-Vec3::Y,Vec3::new(1.,1.,0.).normalize()] {
                let a=blockage(&nav,point-Vec3::Z*0.4,direction);
                let b=blockage(&reference,point-Vec3::Z*0.4,direction);
                assert_eq!(a.map(|v|(v.low,v.ahead)),b.map(|v|(v.low,v.ahead)));
            }
        }}
    }
    #[test] fn navigation_query_work_is_local_in_a_dense_area() {
        let mut area=area(Vec::new());
        for x in -70..70 {for y in -70..70 {
            let x=x as f32*2.;let y=y as f32*2.;
            area.triangles.push([Vec3::new(x,y,0.),Vec3::new(x+1.,y,0.),Vec3::new(x+1.,y,0.3)]
                .map(coords::to_skate).map(|v|v.to_array()));
        }}
        let nav=Navigation::new(&area);let reference=unindexed(&nav);
        assert!(nav.nearby(Vec3::ZERO,6.).count()<area.triangles.len()/50);
        let run=|nav:&Navigation|{
            let started=std::time::Instant::now();
            for i in 0..2000 {
                let p=Vec3::new((i%8) as f32*0.1,0.1,0.);
                std::hint::black_box(blockage(nav,std::hint::black_box(p),Vec3::Y));
            }
            started.elapsed().as_secs_f64()*1000.
        };
        let full=run(&reference);let indexed=run(&nav);
        eprintln!("AI navigation benchmark: triangles={} full_scan_ms={full:.3} indexed_ms={indexed:.3} queries=2000",area.triangles.len());
        // Assert bounded query work, not machine-dependent benchmark timing.
    }
    #[test] fn obstacles_distinguish_low_curbs_from_tall_walls() {
        let wall=|height|world::build_area(Vec3::ZERO,[
            [[-3.,2.,0.],[3.,2.,0.],[3.,2.,height]],
            [[-3.,2.,0.],[3.,2.,height],[-3.,2.,height]],
        ].into_iter());
        assert!(blockage(&Navigation::new(&wall(0.25)),Vec3::ZERO,Vec3::Y).unwrap().low);
        assert!(!blockage(&Navigation::new(&wall(4.)),Vec3::ZERO,Vec3::Y).unwrap().low);
        assert!(blockage(&Navigation::new(&wall(0.25)),Vec3::ZERO,-Vec3::Y).is_none());
    }
    #[test] fn small_steering_clears_deadzone_and_sharp_turns_use_diagonals() {
        assert!(steering_pad(0.1)[0].abs()>8192);
        assert_eq!(steering_pad(0.),[0,0]);
        assert!(steering_pad(1.5)[1]<0&&steering_pad(-1.5)[0]>0);
    }
    #[test] fn moving_waypoints_do_not_trigger_arrival_braking() {
        for gap in [1.,2.,4.,5.] {
            assert_eq!(cruising_speed(7.,gap,0.),7.);
            assert!(cruising_speed(7.,gap,0.8)>6.);
        }
        assert_eq!(cruising_speed(0.,1.,0.),0.);
        assert!(cruising_speed(0.,3.,0.)<cruising_speed(0.,10.,0.));
        assert!(cruising_speed(7.,4.,2.8)<4.);
    }
    fn area(rails: Vec<Vec<[f32;3]>>) -> world::Area {
        let mut area=world::build_area(Vec3::ZERO,[[[0.,0.,0.],[1.,0.,0.],[0.,1.,0.]]].into_iter());
        area.rails=rails;area
    }
    #[test] fn only_approachable_low_grind_lines_are_selected() {
        let line=|a:Vec3,b:Vec3|vec![coords::to_skate(a).to_array(),coords::to_skate(b).to_array()];
        let good=line(Vec3::new(0.,5.,0.5),Vec3::new(0.,12.,0.5));
        assert!(rail_line(&area(vec![good]),Vec3::ZERO,Vec3::Y).is_some());
        for (a,b) in [(Vec3::new(0.,5.,4.),Vec3::new(0.,12.,4.)),
            (Vec3::new(0.,-5.,0.5),Vec3::new(0.,-12.,0.5)),
            (Vec3::new(8.,5.,0.5),Vec3::new(8.,12.,0.5))] {
            assert!(rail_line(&area(vec![line(a,b)]),Vec3::ZERO,Vec3::Y).is_none());
        }
    }
    #[test] fn floor_uses_gta_height_and_rejects_other_levels() {
        let area=Navigation::new(&area(Vec::new()));
        assert_eq!(floor(&area,Vec3::new(0.2,0.2,0.5)).map(|v|v.0),Some(0.));
        assert!(floor(&area,Vec3::new(0.2,0.2,10.)).is_none());
        assert!(floor(&area,Vec3::new(4.,4.,0.)).is_none());
    }
    #[test]
    #[ignore = "requires prepared Skate assets and GTA ped cache; no running GTA needed"]
    fn independent_sessions_follow_without_replaying_player_inputs() {
        let root=std::env::var("SKATEV_TEST_DATA_ROOT").expect("SKATEV_TEST_DATA_ROOT");
        let cache=std::env::var("SKATEV_TEST_PED_CACHE").expect("SKATEV_TEST_PED_CACHE");
        let area=Arc::new(world::build_area(Vec3::ZERO,[
            [[-120.,-120.,0.],[120.,-120.,0.],[-120.,120.,0.]],
            [[120.,-120.,0.],[120.,120.,0.],[-120.,120.,0.]],
        ].into_iter()));
        let log:Log=Arc::new(|line|eprintln!("{line}"));
        let request=|id,spawn| Request{id,entity:id,spawn,heading:0.,character:CharacterRequest{
            cache_root:cache.clone().into(),model_hash:crate::ped::joaat("player_zero"),
            variation:crate::ped::Variation{drawable:[0;12],texture:[0;12]},budget:0,live_skeleton:None}};
        let mut a=Agent::new(Path::new(&root),area.clone(),request(1,Vec3::ZERO),&log).unwrap();
        let mut b=Agent::new(Path::new(&root),area.clone(),request(2,Vec3::new(12.,0.,0.)),&log).unwrap();
        a.goal=Goal{id:1,at:Vec3::new(0.,20.,0.),current:Vec3::ZERO,speed:0.,tricks:false};
        b.goal=Goal{id:2,at:Vec3::new(12.,-20.,0.),current:Vec3::new(12.,0.,0.),speed:0.,tricks:false};
        let mut last=None;
        for tick in 0..720 {
            let sa=a.step(Some(&area),&log).unwrap().unwrap();
            let sb=b.step(Some(&area),&log).unwrap().unwrap();
            assert!(!sa.bones.is_empty()&&!sb.bones.is_empty());
            assert!(sa.board_entity.is_finite()&&sb.board_entity.is_finite());
            if tick%120==0 {eprintln!("route tick {tick}: a={:?} b={:?} bvel={:?}",sa.root,sb.root,sb.velocity);}
            last=Some((sa,sb));
        }
        let (sa,sb)=last.as_ref().unwrap();
        eprintln!("AI route test: a={:?}, b={:?}",sa.root,sb.root);
        assert!(distance(sa.root,Vec3::new(0.,20.,0.))<3.,"first agent did not follow its own route");
        assert!(distance(sb.root,Vec3::new(12.,-20.,0.))<3.,"second agent did not follow its own route");
        // Both leaders turn ninety degrees and keep moving. The two sessions
        // must steer in opposite directions without sharing their pad history.
        for tick in 0..900 {
            let along=(tick as f32+1.)/60.*2.;
            a.goal.at=Vec3::new(along,20.,0.);a.goal.speed=2.;
            b.goal.at=Vec3::new(12.-along,-20.,0.);b.goal.speed=2.;
            let sa=a.step(Some(&area),&log).unwrap().unwrap();
            let sb=b.step(Some(&area),&log).unwrap().unwrap();
            last=Some((sa,sb));
        }
        let (sa,sb)=last.as_ref().unwrap();
        eprintln!("AI moving turn test: a={:?}, b={:?}",sa.root,sb.root);
        assert!(distance(sa.root,a.goal.at)<8.,"first agent lost a moving leader after the corner");
        assert!(distance(sb.root,b.goal.at)<8.,"second agent lost its independent moving leader");
        // Match the host's short steering lookahead at a real skating speed.
        // This previously capped speed near 3 m/s regardless of leader speed.
        let mut brakes=0;
        let start=sa.root;
        for _ in 0..600 {
            let current=last.as_ref().unwrap().0.root;
            a.goal.at=current+Vec3::X*4.;a.goal.speed=7.;
            let sa=a.step(Some(&area),&log).unwrap().unwrap();
            if a.braking {brakes+=1;}
            last=Some((sa,last.as_ref().unwrap().1.clone()));
        }
        let end=&last.as_ref().unwrap().0;
        eprintln!("AI cruising test: travelled={:.2} speed={:.2} braking_ticks={brakes}",distance(start,end.root),horizontal(end.velocity).length());
        assert!(distance(start,end.root)>50.,"short route waypoints prevented cruising");
        assert!(brakes<120,"cruising repeatedly applied the brake");
        let stop=end.root+Vec3::X*8.;
        a.goal.at=stop;a.goal.speed=0.;
        for _ in 0..600 {
            let sa=a.step(Some(&area),&log).unwrap().unwrap();
            last=Some((sa,last.as_ref().unwrap().1.clone()));
        }
        assert!(distance(last.as_ref().unwrap().0.root,stop)<3.,"follower failed to stop after cruising");
        let curb=Arc::new(world::build_area(Vec3::ZERO,[
            [[-10.,-20.,0.],[10.,-20.,0.],[-10.,6.,0.]],
            [[10.,-20.,0.],[10.,6.,0.],[-10.,6.,0.]],
            [[-10.,6.,0.22],[10.,6.,0.22],[-10.,30.,0.22]],
            [[10.,6.,0.22],[10.,30.,0.22],[-10.,30.,0.22]],
            [[-10.,6.,0.],[10.,6.,0.],[10.,6.,0.22]],
            [[-10.,6.,0.],[10.,6.,0.22],[-10.,6.,0.22]],
        ].into_iter()));
        let mut c=Agent::new(Path::new(&root),curb.clone(),request(3,Vec3::ZERO),&log).unwrap();
        c.goal=Goal{id:3,at:Vec3::new(0.,20.,0.22),current:Vec3::ZERO,speed:0.,tricks:false};
        let mut cs=None;
        for _ in 0..900 {cs=c.step(Some(&curb),&log).unwrap();}
        let cs=cs.unwrap();
        eprintln!("AI curb test: position={:?}",cs.root);
        assert!(cs.root.y>10.,"follower kept pushing into the curb instead of clearing it");
        assert_eq!(c.session.natural_stance().0,0,"follower did not use regular stance");
        // A remote player-area update must not remove the follower's floor.
        let mut remote=(*curb).clone();remote.center=Vec3::new(600.,0.,0.);
        let remote=Arc::new(remote);
        let before=c.step(Some(&remote),&log).unwrap().unwrap();
        assert!(Arc::ptr_eq(&c.area,&curb));
        // If the usable region actually ends, publish a live waiting snapshot
        // in place rather than None (which used to restore GTA teleport AI).
        let mut missing=(*curb).clone();missing.center=Vec3::new(-600.,0.,0.);
        c.area=Arc::new(missing);c.route_areas.clear();
        let waiting=c.step(None,&log).unwrap().unwrap();
        assert!(distance(waiting.root,before.root)<0.001);
        assert!(waiting.tick>before.tick);
        let wall=Arc::new(world::build_area(Vec3::ZERO,[
            [[-30.,-20.,0.],[30.,-20.,0.],[-30.,30.,0.]],
            [[30.,-20.,0.],[30.,30.,0.],[-30.,30.,0.]],
            [[-1.,6.,0.],[1.,6.,0.],[1.,6.,4.]],
            [[-1.,6.,0.],[1.,6.,4.],[-1.,6.,4.]],
        ].into_iter()));
        let mut d=Agent::new(Path::new(&root),wall.clone(),request(4,Vec3::ZERO),&log).unwrap();
        d.goal=Goal{id:4,at:Vec3::new(0.,20.,0.),current:Vec3::ZERO,speed:0.,tricks:false};
        let mut ds=None;
        for _ in 0..900 {ds=d.step(Some(&wall),&log).unwrap();}
        let ds=ds.unwrap();
        eprintln!("AI tall obstacle test: position={:?}",ds.root);
        assert!(ds.root.y>10.,"follower failed to detour around a tall obstacle");
        // Streaming preparation must happen in the background while ownership
        // and simulation continue. Move only the old region's metadata, not
        // the actor; the overlapping replacement still covers its feet.
        let mut old=(*wall).clone();old.center=ds.root-Vec3::X*(world::RADIUS-30.);
        d.area=Arc::new(old);d.packet=(d.packet+29)/30*30;
        let before=d.step(Some(&wall),&log).unwrap().unwrap();
        assert!(d.pending_collision.is_some());
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(5);
        while !Arc::ptr_eq(&d.area,&wall)&&std::time::Instant::now()<deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
            let snapshot=d.step(Some(&wall),&log).unwrap().unwrap();
            assert!(snapshot.tick>before.tick);
            assert!(distance(snapshot.root,before.root)<10.,"background region install moved the actor");
        }
        assert!(Arc::ptr_eq(&d.area,&wall),"background collision never installed");
    }
}
