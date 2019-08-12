use crate::{
    resource::{NavVec3, ZERO_TRESHOLD},
    Scalar,
};
use core::id::ID;
use petgraph::{algo::astar, graph::NodeIndex, visit::EdgeRef, Graph, Undirected};
use serde::{Deserialize, Serialize};
use spade::{rtree::RTree, BoundingRect, SpatialObject};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    result::Result as StdResult,
};

pub type NavMeshID = ID<NavMesh>;

#[derive(Debug, Clone)]
pub enum Error {
    /// (triangle index, local vertice index, global vertice index)
    TriangleVerticeIndexOutOfBounds(u32, u8, u32),
}

pub type NavResult<T> = StdResult<T, Error>;

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Serialize, Deserialize)]
pub struct NavTriangle {
    pub first: u32,
    pub second: u32,
    pub third: u32,
}

impl From<(u32, u32, u32)> for NavTriangle {
    fn from(value: (u32, u32, u32)) -> Self {
        Self {
            first: value.0,
            second: value.1,
            third: value.2,
        }
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NavArea {
    pub triangle: u32,
    pub size: Scalar,
    pub cost: Scalar,
    pub inv_cost: Scalar,
    pub center: NavVec3,
    pub radius: Scalar,
    pub radius_sqr: Scalar,
}

impl NavArea {
    #[inline]
    pub fn calculate_area(a: NavVec3, b: NavVec3, c: NavVec3) -> Scalar {
        let ab = b - a;
        let ac = c - a;
        ab.cross(ac).magnitude() * 0.5
    }

    #[inline]
    pub fn calculate_center(a: NavVec3, b: NavVec3, c: NavVec3) -> NavVec3 {
        let v = a + b + c;
        NavVec3::new(v.x / 3.0, v.y / 3.0, v.z / 3.0)
    }
}

#[derive(Debug, Default, Copy, Clone, Eq, Serialize, Deserialize)]
struct NavConnection(pub u32, pub u32);

impl Hash for NavConnection {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let first = self.0.min(self.1);
        let second = self.0.max(self.1);
        first.hash(state);
        second.hash(state);
    }
}

impl PartialEq for NavConnection {
    fn eq(&self, other: &Self) -> bool {
        let first = self.0.min(self.1);
        let second = self.0.max(self.1);
        let ofirst = other.0.min(other.1);
        let osecond = other.0.max(other.1);
        first == ofirst && second == osecond
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NavSpatialObject {
    pub index: usize,
    pub a: NavVec3,
    pub b: NavVec3,
    pub c: NavVec3,
    ab: NavVec3,
    bc: NavVec3,
    ca: NavVec3,
    normal: NavVec3,
    dab: NavVec3,
    dbc: NavVec3,
    dca: NavVec3,
}

impl NavSpatialObject {
    pub fn new(index: usize, a: NavVec3, b: NavVec3, c: NavVec3) -> Self {
        let ab = b - a;
        let bc = c - b;
        let ca = a - c;
        let normal = (a - b).cross(a - c).normalize();
        let dab = normal.cross(ab);
        let dbc = normal.cross(bc);
        let dca = normal.cross(ca);
        Self {
            index,
            a,
            b,
            c,
            ab,
            bc,
            ca,
            normal,
            dab,
            dbc,
            dca,
        }
    }

    #[inline]
    pub fn normal(&self) -> NavVec3 {
        self.normal
    }

    pub fn closest_point(&self, point: NavVec3) -> NavVec3 {
        let pab = point.project(self.a, self.b);
        let pbc = point.project(self.b, self.c);
        let pca = point.project(self.c, self.a);
        if pca > 1.0 && pab < 0.0 {
            return self.a;
        } else if pab > 1.0 && pbc < 0.0 {
            return self.b;
        } else if pbc > 1.0 && pca < 0.0 {
            return self.c;
        } else if pab >= 0.0 && pab <= 1.0 && !point.is_above_plane(self.a, self.dab) {
            return NavVec3::unproject(self.a, self.b, pab);
        } else if pbc >= 0.0 && pbc <= 1.0 && !point.is_above_plane(self.b, self.dbc) {
            return NavVec3::unproject(self.b, self.c, pbc);
        } else if pca >= 0.0 && pca <= 1.0 && !point.is_above_plane(self.c, self.dca) {
            return NavVec3::unproject(self.c, self.a, pca);
        }
        point.project_on_plane(self.a, self.normal)
    }
}

impl SpatialObject for NavSpatialObject {
    type Point = NavVec3;

    fn mbr(&self) -> BoundingRect<Self::Point> {
        let min = NavVec3::new(
            self.a.x.min(self.b.x).min(self.c.x),
            self.a.y.min(self.b.y).min(self.c.y),
            self.a.z.min(self.b.z).min(self.c.z),
        );
        let max = NavVec3::new(
            self.a.x.max(self.b.x).max(self.c.x),
            self.a.y.max(self.b.y).max(self.c.y),
            self.a.z.max(self.b.z).max(self.c.z),
        );
        BoundingRect::from_corners(&min, &max)
    }

    fn distance2(&self, point: &Self::Point) -> Scalar {
        (*point - self.closest_point(*point)).sqr_magnitude()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NavQuery {
    Accuracy,
    Closest,
    ClosestFirst,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NavPathMode {
    Accuracy,
    MidPoints,
}

#[derive(Debug, Default)]
pub struct NavMeshesRes(pub(crate) HashMap<NavMeshID, NavMesh>);

impl NavMeshesRes {
    #[inline]
    pub fn register(&mut self, mesh: NavMesh) -> NavMeshID {
        let id = mesh.id();
        self.0.insert(id, mesh);
        id
    }

    #[inline]
    pub fn unregister(&mut self, id: NavMeshID) -> Option<NavMesh> {
        self.0.remove(&id)
    }

    #[inline]
    pub fn unregister_all(&mut self) {
        self.0.clear()
    }

    #[inline]
    pub fn meshes_iter(&self) -> impl Iterator<Item = &NavMesh> {
        self.0.values()
    }

    #[inline]
    pub fn find_mesh(&self, id: NavMeshID) -> Option<&NavMesh> {
        self.0.get(&id)
    }

    #[inline]
    pub fn find_mesh_mut(&mut self, id: NavMeshID) -> Option<&mut NavMesh> {
        self.0.get_mut(&id)
    }

    pub fn closest_point(&self, point: NavVec3, query: NavQuery) -> Option<(NavMeshID, NavVec3)> {
        self.0
            .iter()
            .filter_map(|(id, mesh)| {
                mesh.closest_point(point, query)
                    .map(|p| (p, (p - point).sqr_magnitude(), *id))
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(p, _, id)| (id, p))
    }
}

#[derive(Debug, Default, Clone)]
pub struct NavMesh {
    id: NavMeshID,
    vertices: Vec<NavVec3>,
    triangles: Vec<NavTriangle>,
    areas: Vec<NavArea>,
    // {triangle connection: (distance sqr, vertex connection)}
    connections: HashMap<NavConnection, (Scalar, NavConnection)>,
    graph: Graph<(), Scalar, Undirected>,
    nodes: Vec<NodeIndex>,
    nodes_map: HashMap<NodeIndex, usize>,
    rtree: RTree<NavSpatialObject>,
    spatials: Vec<NavSpatialObject>,
    // {triangle index: [(from, to)]}
    hard_edges: HashMap<usize, Vec<(NavVec3, NavVec3)>>,
}

impl NavMesh {
    pub fn new(vertices: Vec<NavVec3>, triangles: Vec<NavTriangle>) -> NavResult<Self> {
        let areas = triangles
            .iter()
            .enumerate()
            .map(|(i, triangle)| {
                if triangle.first >= vertices.len() as u32 {
                    return Err(Error::TriangleVerticeIndexOutOfBounds(
                        i as u32,
                        0,
                        triangle.first,
                    ));
                }
                if triangle.second >= vertices.len() as u32 {
                    return Err(Error::TriangleVerticeIndexOutOfBounds(
                        i as u32,
                        1,
                        triangle.second,
                    ));
                }
                if triangle.third >= vertices.len() as u32 {
                    return Err(Error::TriangleVerticeIndexOutOfBounds(
                        i as u32,
                        2,
                        triangle.third,
                    ));
                }
                let first = vertices[triangle.first as usize];
                let second = vertices[triangle.second as usize];
                let third = vertices[triangle.third as usize];
                let center = NavArea::calculate_center(first, second, third);
                let radius = (first - center)
                    .magnitude()
                    .max((second - center).magnitude())
                    .max((third - center).magnitude());
                Ok(NavArea {
                    triangle: i as u32,
                    size: NavArea::calculate_area(first, second, third),
                    cost: 1.0,
                    inv_cost: 1.0,
                    center,
                    radius,
                    radius_sqr: radius * radius,
                })
            })
            .collect::<NavResult<Vec<_>>>()?;

        // {edge: [triangle index]}
        let mut edges = HashMap::<NavConnection, Vec<usize>>::with_capacity(triangles.len() * 3);
        for (index, triangle) in triangles.iter().enumerate() {
            let edge_a = NavConnection(triangle.first, triangle.second);
            let edge_b = NavConnection(triangle.second, triangle.third);
            let edge_c = NavConnection(triangle.third, triangle.first);
            if let Some(tris) = edges.get_mut(&edge_a) {
                tris.push(index);
            } else {
                edges.insert(edge_a, vec![index]);
            }
            if let Some(tris) = edges.get_mut(&edge_b) {
                tris.push(index);
            } else {
                edges.insert(edge_b, vec![index]);
            }
            if let Some(tris) = edges.get_mut(&edge_c) {
                tris.push(index);
            } else {
                edges.insert(edge_c, vec![index]);
            }
        }

        let connections = edges
            .iter()
            .flat_map(|(verts, tris)| {
                let mut result = HashMap::with_capacity(tris.len() * tris.len());
                for a in tris {
                    for b in tris {
                        if a != b {
                            result.insert(NavConnection(*a as u32, *b as u32), *verts);
                        }
                    }
                }
                result
            })
            .collect::<HashMap<_, _>>()
            .into_iter()
            .map(|(tri_conn, vert_conn)| {
                let a = areas[tri_conn.0 as usize].center;
                let b = areas[tri_conn.1 as usize].center;
                let weight = (b - a).sqr_magnitude();
                (tri_conn, (weight, vert_conn))
            })
            .collect::<HashMap<_, _>>();

        let mut graph = Graph::<(), Scalar, Undirected>::new_undirected();
        let nodes = (0..triangles.len())
            .map(|_| graph.add_node(()))
            .collect::<Vec<_>>();
        graph.extend_with_edges(
            connections
                .iter()
                .map(|(conn, (w, _))| (nodes[conn.0 as usize], nodes[conn.1 as usize], w)),
        );
        let nodes_map = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();

        let spatials = triangles
            .iter()
            .enumerate()
            .map(|(index, triangle)| {
                NavSpatialObject::new(
                    index,
                    vertices[triangle.first as usize],
                    vertices[triangle.second as usize],
                    vertices[triangle.third as usize],
                )
            })
            .collect::<Vec<_>>();

        let mut rtree = RTree::new();
        for spatial in &spatials {
            rtree.insert(spatial.clone());
        }

        let hard_edges = triangles
            .iter()
            .enumerate()
            .filter_map(|(index, triangle)| {
                let edge_a = NavConnection(triangle.first, triangle.second);
                let edge_b = NavConnection(triangle.second, triangle.third);
                let edge_c = NavConnection(triangle.third, triangle.first);
                let mut planes = vec![];
                if edges[&edge_a].len() < 2 {
                    planes.push((
                        vertices[triangle.first as usize],
                        vertices[triangle.second as usize],
                    ));
                }
                if edges[&edge_b].len() < 2 {
                    planes.push((
                        vertices[triangle.second as usize],
                        vertices[triangle.third as usize],
                    ));
                }
                if edges[&edge_c].len() < 2 {
                    planes.push((
                        vertices[triangle.third as usize],
                        vertices[triangle.first as usize],
                    ));
                }
                if planes.is_empty() {
                    None
                } else {
                    Some((index, planes))
                }
            })
            .collect::<HashMap<_, _>>();

        Ok(Self {
            id: ID::new(),
            vertices,
            triangles,
            areas,
            connections,
            graph,
            nodes,
            nodes_map,
            rtree,
            spatials,
            hard_edges,
        })
    }

    #[inline]
    pub fn id(&self) -> NavMeshID {
        self.id
    }

    #[inline]
    pub fn vertices(&self) -> &[NavVec3] {
        &self.vertices
    }

    #[inline]
    pub fn triangles(&self) -> &[NavTriangle] {
        &self.triangles
    }

    #[inline]
    pub fn areas(&self) -> &[NavArea] {
        &self.areas
    }

    #[inline]
    pub fn set_area_cost(&mut self, index: usize, cost: Scalar) -> Scalar {
        let area = &mut self.areas[index];
        let old = area.cost;
        let cost = cost.max(0.0);
        area.cost = cost;
        area.inv_cost = if cost.abs() == 0.0 { 0.0 } else { 1.0 / cost };
        old
    }

    pub fn closest_point(&self, point: NavVec3, query: NavQuery) -> Option<NavVec3> {
        self.find_closest_triangle(point, query)
            .map(|triangle| self.spatials[triangle].closest_point(point))
    }

    pub fn find_path(
        &self,
        from: NavVec3,
        to: NavVec3,
        query: NavQuery,
        mode: NavPathMode,
    ) -> Option<Vec<NavVec3>> {
        if (to - from).sqr_magnitude() < ZERO_TRESHOLD {
            return None;
        }
        let start = if let Some(start) = self.find_closest_triangle(from, query) {
            start
        } else {
            return None;
        };
        let end = if let Some(end) = self.find_closest_triangle(to, query) {
            end
        } else {
            return None;
        };
        let from = self.spatials[start].closest_point(from);
        let to = self.spatials[end].closest_point(to);
        if let Some((triangles, _)) = self.find_path_triangles(start, end) {
            if triangles.is_empty() {
                return None;
            } else if triangles.len() == 1 {
                return Some(vec![from, to]);
            }
            match mode {
                NavPathMode::Accuracy => {
                    return Some(self.find_path_accuracy(from, to, &triangles));
                }
                NavPathMode::MidPoints => {
                    return Some(self.find_path_midpoints(from, to, &triangles));
                }
            }
        }
        None
    }

    fn find_path_accuracy(&self, from: NavVec3, to: NavVec3, triangles: &[usize]) -> Vec<NavVec3> {
        enum Node {
            Point(NavVec3),
            // (a, b, normal)
            LevelChange(NavVec3, NavVec3, NavVec3),
        }

        // TODO: reduce allocations.
        if triangles.len() == 2 {
            let NavConnection(a, b) =
                self.connections[&NavConnection(triangles[0] as u32, triangles[1] as u32)].1;
            let a = self.vertices[a as usize];
            let b = self.vertices[b as usize];
            let n = self.spatials[triangles[0]].normal();
            let m = self.spatials[triangles[1]].normal();
            if !NavVec3::is_line_between_points(from, to, a, b, n) {
                let da = (from - a).sqr_magnitude();
                let db = (from - b).sqr_magnitude();
                let point = if da < db { a } else { b };
                return vec![from, point, to];
            } else if n.dot(m) < 1.0 - ZERO_TRESHOLD {
                let n = (b - a).normalize().cross(n);
                if let Some(point) = NavVec3::raycast_line(from, to, a, b, n) {
                    return vec![from, point, to];
                }
            }
            return vec![from, to];
        }
        let mut start = from;
        let mut last_normal = self.spatials[triangles[0]].normal();
        let mut nodes = Vec::with_capacity(triangles.len() - 1);
        for triplets in triangles.windows(3) {
            let NavConnection(a, b) =
                self.connections[&NavConnection(triplets[0] as u32, triplets[1] as u32)].1;
            let a = self.vertices[a as usize];
            let b = self.vertices[b as usize];
            let NavConnection(c, d) =
                self.connections[&NavConnection(triplets[1] as u32, triplets[2] as u32)].1;
            let c = self.vertices[c as usize];
            let d = self.vertices[d as usize];
            let n = self.spatials[triplets[1]].normal();
            let old_last_normal = last_normal;
            last_normal = n;
            if !NavVec3::is_line_between_points(start, c, a, b, n)
                || !NavVec3::is_line_between_points(start, d, a, b, n)
            {
                let da = (start - a).sqr_magnitude();
                let db = (start - b).sqr_magnitude();
                start = if da < db { a } else { b };
                nodes.push(Node::Point(start));
            } else if old_last_normal.dot(n) < 1.0 - ZERO_TRESHOLD {
                let n = (b - a).normalize().cross(n);
                nodes.push(Node::LevelChange(a, b, n));
            }
        }
        {
            let NavConnection(a, b) = self.connections[&NavConnection(
                triangles[triangles.len() - 2] as u32,
                triangles[triangles.len() - 1] as u32,
            )]
                .1;
            let a = self.vertices[a as usize];
            let b = self.vertices[b as usize];
            let n = self.spatials[triangles[triangles.len() - 2]].normal();
            let m = self.spatials[triangles[triangles.len() - 1]].normal();
            if !NavVec3::is_line_between_points(start, to, a, b, n) {
                let da = (start - a).sqr_magnitude();
                let db = (start - b).sqr_magnitude();
                let point = if da < db { a } else { b };
                nodes.push(Node::Point(point));
            } else if n.dot(m) < 1.0 - ZERO_TRESHOLD {
                let n = (b - a).normalize().cross(n);
                nodes.push(Node::LevelChange(a, b, n));
            }
        }

        let mut points = Vec::with_capacity(nodes.len() + 2);
        points.push(from);
        let mut point = from;
        for i in 0..nodes.len() {
            match nodes[i] {
                Node::Point(p) => {
                    point = p;
                    points.push(p);
                }
                Node::LevelChange(a, b, n) => {
                    let next = nodes
                        .iter()
                        .skip(i + 1)
                        .find_map(|n| match n {
                            Node::Point(p) => Some(*p),
                            _ => None,
                        })
                        .unwrap_or(to);
                    if let Some(p) = NavVec3::raycast_line(point, next, a, b, n) {
                        points.push(p);
                    }
                }
            }
        }
        points.push(to);
        points.dedup();
        points
    }

    fn find_path_midpoints(&self, from: NavVec3, to: NavVec3, triangles: &[usize]) -> Vec<NavVec3> {
        if triangles.len() == 2 {
            let NavConnection(a, b) =
                self.connections[&NavConnection(triangles[0] as u32, triangles[1] as u32)].1;
            let a = self.vertices[a as usize];
            let b = self.vertices[b as usize];
            let n = self.spatials[triangles[0]].normal();
            let m = self.spatials[triangles[1]].normal();
            if n.dot(m) < 1.0 - ZERO_TRESHOLD || !NavVec3::is_line_between_points(from, to, a, b, n)
            {
                return vec![from, (a + b) * 0.5, to];
            } else {
                return vec![from, to];
            }
        }
        let mut start = from;
        let mut last_normal = self.spatials[triangles[0]].normal();
        let mut points = Vec::with_capacity(triangles.len() + 1);
        points.push(from);
        for triplets in triangles.windows(3) {
            let NavConnection(a, b) =
                self.connections[&NavConnection(triplets[0] as u32, triplets[1] as u32)].1;
            let a = self.vertices[a as usize];
            let b = self.vertices[b as usize];
            let point = (a + b) * 0.5;
            let n = self.spatials[triplets[1]].normal();
            let old_last_normal = last_normal;
            last_normal = n;
            if old_last_normal.dot(n) < 1.0 - ZERO_TRESHOLD {
                start = point;
                points.push(start);
            } else {
                let NavConnection(c, d) =
                    self.connections[&NavConnection(triplets[1] as u32, triplets[2] as u32)].1;
                let c = self.vertices[c as usize];
                let d = self.vertices[d as usize];
                let end = (c + d) * 0.5;
                if !NavVec3::is_line_between_points(start, end, a, b, n) {
                    start = point;
                    points.push(start);
                }
            }
        }
        {
            let NavConnection(a, b) = self.connections[&NavConnection(
                triangles[triangles.len() - 2] as u32,
                triangles[triangles.len() - 1] as u32,
            )]
                .1;
            let a = self.vertices[a as usize];
            let b = self.vertices[b as usize];
            let n = self.spatials[triangles[triangles.len() - 2]].normal();
            let m = self.spatials[triangles[triangles.len() - 1]].normal();
            if n.dot(m) < 1.0 - ZERO_TRESHOLD
                || !NavVec3::is_line_between_points(start, to, a, b, n)
            {
                points.push((a + b) * 0.5);
            }
        }
        points.push(to);
        points.dedup();
        points
    }

    #[inline]
    pub fn find_path_triangles(&self, from: usize, to: usize) -> Option<(Vec<usize>, Scalar)> {
        let to = self.nodes[to];
        astar(
            &self.graph,
            self.nodes[from],
            |n| n == to,
            |e| {
                let a = self.areas[self.nodes_map[&e.source()]].cost;
                let b = self.areas[self.nodes_map[&e.target()]].cost;
                *e.weight() * a * b
            },
            |_| 0.0,
        )
        .map(|(c, v)| (v.iter().map(|v| self.nodes_map[&v]).collect(), c))
    }

    pub fn find_closest_triangle(&self, point: NavVec3, query: NavQuery) -> Option<usize> {
        match query {
            NavQuery::Accuracy => self.rtree.nearest_neighbor(&point).map(|t| t.index),
            NavQuery::ClosestFirst => self.rtree.close_neighbor(&point).map(|t| t.index),
            NavQuery::Closest => self
                .rtree
                .nearest_neighbors(&point)
                .into_iter()
                .map(|o| (o.distance2(&point), o))
                .fold(None, |a: Option<(Scalar, &NavSpatialObject)>, i| {
                    if let Some(a) = a {
                        if i.0 < a.0 {
                            Some(i)
                        } else {
                            Some(a)
                        }
                    } else {
                        Some(i)
                    }
                })
                .map(|(_, t)| t.index),
        }
    }

    pub fn path_target_point(
        path: &[NavVec3],
        point: NavVec3,
        offset: Scalar,
    ) -> Option<(NavVec3, Scalar)> {
        let s = Self::project_on_path(path, point, offset);
        if let Some(p) = Self::point_on_path(path, s) {
            Some((p, s))
        } else {
            None
        }
    }

    pub fn project_on_path(path: &[NavVec3], point: NavVec3, offset: Scalar) -> Scalar {
        let p = match path.len() {
            0 | 1 => 0.0,
            2 => Self::project_on_line(path[0], path[1], point),
            _ => {
                path.windows(2)
                    .scan(0.0, |state, pair| {
                        let dist = *state;
                        *state += (pair[1] - pair[0]).magnitude();
                        Some((dist, pair))
                    })
                    .map(|(dist, pair)| {
                        let (p, s) = Self::point_on_line(pair[0], pair[1], point);
                        (dist + s, (p - point).sqr_magnitude())
                    })
                    .min_by(|(_, a), (_, b)| a.partial_cmp(&b).unwrap())
                    .unwrap()
                    .0
            }
        };
        (p + offset).max(0.0).min(Self::path_length(path))
    }

    pub fn point_on_path(path: &[NavVec3], mut s: Scalar) -> Option<NavVec3> {
        match path.len() {
            0 | 1 => None,
            2 => Some(NavVec3::unproject(
                path[0],
                path[1],
                s / Self::path_length(path),
            )),
            _ => {
                for pair in path.windows(2) {
                    let d = (pair[1] - pair[0]).magnitude();
                    if s <= d {
                        return Some(NavVec3::unproject(pair[0], pair[1], s / d));
                    }
                    s -= d;
                }
                None
            }
        }
    }

    pub fn path_length(path: &[NavVec3]) -> Scalar {
        match path.len() {
            0 | 1 => 0.0,
            2 => (path[1] - path[0]).magnitude(),
            _ => path
                .windows(2)
                .fold(0.0, |a, pair| a + (pair[1] - pair[0]).magnitude()),
        }
    }

    fn project_on_line(from: NavVec3, to: NavVec3, point: NavVec3) -> Scalar {
        let d = (to - from).magnitude();
        let p = point.project(from, to);
        d * p
    }

    fn point_on_line(from: NavVec3, to: NavVec3, point: NavVec3) -> (NavVec3, Scalar) {
        let d = (to - from).magnitude();
        let p = point.project(from, to);
        if p <= 0.0 {
            (from, 0.0)
        } else if p >= 1.0 {
            (to, d)
        } else {
            (NavVec3::unproject(from, to, p), p * d)
        }
    }
}
