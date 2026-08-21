// The node-set glue of the OCCT seam (docs/03 §The OCCT seam as built;
// v0.1 item 3 WP-C). The fork (bencbartlett/opencascade-rs, branch `cicada`)
// carries the FIRST glue — box, prism, cut, canonical bytes, tessellate,
// the exception boundary's self-test; this header carries the rest of the
// kernel operations the stdlib's Solid nodes need, compiled by cicada-geom's
// own build.rs with cxx-build against the same prebuilt OCCT (DEP_OCCT_ROOT)
// the fork links. Same rules as the fork's cicada.hxx:
//
//   * every function is declared `-> Result<T>` in glue.rs, so any OCCT
//     Standard_Failure / std::exception / other value it raises arrives in
//     Rust as Err(cxx::Exception) through the trycatch hook below, never as
//     a process abort;
//   * failures OCCT reports by STATUS (an unfinished boolean, a builder that
//     is not done) are turned into throws here, so Rust sees one shape of
//     failure;
//   * nothing here writes an OCCT global (the sharing model, occt/mod.rs):
//     the one subsystem that keeps mutable globals, the STEP translators'
//     statics and the messenger, is reached only through step_read /
//     step_write / quiet_messenger, which Rust serializes under one lock.
//
// The functions live in namespace cicada_geom (glue.rs sets it) so they
// can never collide with the fork's global `cicada_*` symbols.
#pragma once

#include "rust/cxx.h"

#include <APIHeaderSection_MakeHeader.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Section.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepBuilderAPI_TransitionMode.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <BRep_Builder.hxx>
#include <BRep_CurveRepresentation.hxx>
#include <BRep_ListIteratorOfListOfCurveRepresentation.hxx>
#include <BRep_TEdge.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GCPnts_TangentialDeflection.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_CurveType.hxx>
#include <Geom_Surface.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Message.hxx>
#include <Message_Gravity.hxx>
#include <Message_Messenger.hxx>
#include <Message_Printer.hxx>
#include <Precision.hxx>
#include <STEPControl_Reader.hxx>
#include <STEPControl_StepModelType.hxx>
#include <STEPControl_Writer.hxx>
#include <ShapeAnalysis_FreeBounds.hxx>
#include <ShapeBuild_Edge.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <Standard_Failure.hxx>
#include <Standard_Handle.hxx>
#include <Standard_Type.hxx>
#include <StepData_ConfParameters.hxx>
#include <StepBasic_Product.hxx>
#include <StepData_StepModel.hxx>
#include <TCollection_HAsciiString.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_HSequenceOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Iterator.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>
#include <TopoDS_Wire.hxx>
#include <UnitsMethods.hxx>
#include <UnitsMethods_LengthUnit.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include <cmath>
#include <cstddef>
#include <cstdint>
#include <exception>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

// The exception boundary — the same hook the fork's bindings_common.hxx
// defines, repeated here because cxx instantiates it per translation unit
// (it is `static`) and this header is compiled into cicada-geom's own
// static library, not the fork's. Standard_Failure does not derive from
// std::exception, so cxx's default handler would let it unwind into Rust
// and abort the process (probe: exit 0xC0000409); the final catch (...)
// keeps the boundary total. Every bridge function in glue.rs is declared
// Result, so every throw below lands here.
namespace rust {
namespace behavior {
template <typename Try, typename Fail> static void trycatch(Try &&func, Fail &&fail) noexcept try {
  func();
} catch (const Standard_Failure &failure) {
  std::string message = failure.DynamicType()->Name();
  const char *text = failure.GetMessageString();
  if (text != nullptr && *text != '\0') {
    message += ": ";
    message += text;
  }
  fail(message);
} catch (const std::exception &e) {
  fail(e.what());
} catch (...) {
  fail("unknown C++ exception");
}
} // namespace behavior
} // namespace rust

namespace cicada_geom {

// --------------------------------------------------------------------------
// Small helpers (not bridged)
// --------------------------------------------------------------------------

inline std::unique_ptr<TopoDS_Shape> boxed(const TopoDS_Shape &shape) {
  return std::unique_ptr<TopoDS_Shape>(new TopoDS_Shape(shape));
}

[[noreturn]] inline void fail(const std::string &what) { throw std::runtime_error(what); }

// A right-handed frame from nine doubles: origin, unit x, unit z. gp_Ax2
// re-orthogonalizes x against z (our frames are already orthonormal) and
// derives y = z ^ x, which is the right-handed y the Rust frame carries.
inline gp_Ax2 frame_of(rust::Slice<const double> f, const char *who) {
  if (f.size() != 9) {
    fail(std::string(who) + ": a frame is 9 doubles (origin, x, z), got " + std::to_string(f.size()));
  }
  return gp_Ax2(gp_Pnt(f[0], f[1], f[2]), gp_Dir(f[6], f[7], f[8]), gp_Dir(f[3], f[4], f[5]));
}

inline void require_done(const BRepBuilderAPI_MakeShape &maker, const char *who) {
  if (!maker.IsDone()) {
    fail(std::string(who) + " did not complete");
  }
}

// The boolean family reports most failures by status; both routes become a
// throw with OCCT's own report.
inline void require_boolean(BRepAlgoAPI_BooleanOperation &op, const char *who) {
  if (op.HasErrors()) {
    std::ostringstream report;
    op.DumpErrors(report);
    fail(std::string(who) + " failed: " + report.str());
  }
  if (!op.IsDone()) {
    fail(std::string(who) + " did not complete");
  }
}

// Merge the coplanar / co-cylindrical faces and the collinear edges a
// boolean leaves split, so the result's topology describes the geometry
// (GH's Solid Union does the same; STEP consumers and later booleans are
// happier with it). Failures throw through the boundary.
inline TopoDS_Shape unified(const TopoDS_Shape &shape) {
  ShapeUpgrade_UnifySameDomain unify(shape, /*UnifyEdges=*/Standard_True, /*UnifyFaces=*/Standard_True,
                                     /*ConcatBSplines=*/Standard_False);
  unify.Build();
  return unify.Shape();
}

inline TopTools_ListOfShape children_of(const TopoDS_Shape &compound) {
  TopTools_ListOfShape list;
  for (TopoDS_Iterator it(compound); it.More(); it.Next()) {
    list.Append(it.Value());
  }
  return list;
}

// A planar face bounded by a wire (OnlyPlane = false, the construction
// path the fork's cicada_extrude_polygon takes — byte-compatible prisms).
inline TopoDS_Face face_of(const TopoDS_Shape &wire, const char *who) {
  if (wire.ShapeType() != TopAbs_WIRE) {
    fail(std::string(who) + ": the profile must be a wire");
  }
  BRepBuilderAPI_MakeFace maker(TopoDS::Wire(wire), /*OnlyPlane=*/Standard_False);
  require_done(maker, who);
  return maker.Face();
}

// Curve records the edge/section encoders write: kind 0 = open polyline,
// 1 = closed polyline (the closing vertex not repeated), 2 = a full circle
// (10 doubles: center, x direction, y direction, radius). `counts` holds
// the number of xyz triples for polylines and 0 for circles; `data` holds
// the doubles in order.
inline void push_point(rust::Vec<double> &data, const gp_Pnt &p) {
  data.push_back(p.X());
  data.push_back(p.Y());
  data.push_back(p.Z());
}

inline bool is_full_circle(const BRepAdaptor_Curve &curve) {
  if (curve.GetType() != GeomAbs_Circle) {
    return false;
  }
  const Standard_Real span = curve.LastParameter() - curve.FirstParameter();
  return std::abs(span - 2.0 * M_PI) <= Precision::Angular() * 10.0;
}

inline void push_circle(rust::Vec<int32_t> &kinds, rust::Vec<uint32_t> &counts, rust::Vec<double> &data,
                        const BRepAdaptor_Curve &curve) {
  const gp_Circ circ = curve.Circle();
  const gp_Ax2 position = circ.Position();
  kinds.push_back(2);
  counts.push_back(0);
  push_point(data, position.Location());
  const gp_Dir x = position.XDirection();
  const gp_Dir y = position.YDirection();
  data.push_back(x.X());
  data.push_back(x.Y());
  data.push_back(x.Z());
  data.push_back(y.X());
  data.push_back(y.Y());
  data.push_back(y.Z());
  data.push_back(circ.Radius());
}

// The points of one edge in the edge's own orientation: the two ends for a
// line, a tangential-deflection discretization for anything else (linear
// and angular deflection as the caller says).
inline std::vector<gp_Pnt> edge_points(const TopoDS_Edge &edge, double linear, double angular) {
  BRepAdaptor_Curve curve(edge);
  std::vector<gp_Pnt> points;
  if (curve.GetType() == GeomAbs_Line) {
    points.push_back(curve.Value(curve.FirstParameter()));
    points.push_back(curve.Value(curve.LastParameter()));
  } else {
    GCPnts_TangentialDeflection discretizer(curve, curve.FirstParameter(), curve.LastParameter(), angular,
                                            linear, /*minimumOfPoints=*/2);
    const Standard_Integer count = discretizer.NbPoints();
    if (count < 2) {
      fail("edge discretization produced fewer than two points");
    }
    points.reserve(static_cast<std::size_t>(count));
    for (Standard_Integer i = 1; i <= count; ++i) {
      points.push_back(discretizer.Value(i));
    }
  }
  if (edge.Orientation() == TopAbs_REVERSED) {
    std::vector<gp_Pnt> reversed(points.rbegin(), points.rend());
    return reversed;
  }
  return points;
}

// One free edge as a curve record: a full circle stays a circle, a line is
// a two-point open polyline, everything else a discretized open polyline.
inline void push_edge(rust::Vec<int32_t> &kinds, rust::Vec<uint32_t> &counts, rust::Vec<double> &data,
                      const TopoDS_Edge &edge, double linear, double angular) {
  BRepAdaptor_Curve curve(edge);
  if (is_full_circle(curve)) {
    push_circle(kinds, counts, data, curve);
    return;
  }
  const std::vector<gp_Pnt> points = edge_points(edge, linear, angular);
  kinds.push_back(0);
  counts.push_back(static_cast<uint32_t>(points.size()));
  for (const gp_Pnt &p : points) {
    push_point(data, p);
  }
}

// A connected wire as ONE curve record: a single full-circle edge stays a
// circle; otherwise the edges are walked in order (BRepTools_WireExplorer
// orients them head to tail) into one polyline, closed when the wire is.
inline void push_wire(rust::Vec<int32_t> &kinds, rust::Vec<uint32_t> &counts, rust::Vec<double> &data,
                      const TopoDS_Wire &wire, double linear, double angular) {
  std::vector<TopoDS_Edge> edges;
  for (BRepTools_WireExplorer it(wire); it.More(); it.Next()) {
    edges.push_back(it.Current());
  }
  if (edges.empty()) {
    fail("a section wire has no edges");
  }
  if (edges.size() == 1) {
    BRepAdaptor_Curve curve(edges[0]);
    if (is_full_circle(curve)) {
      push_circle(kinds, counts, data, curve);
      return;
    }
  }
  std::vector<gp_Pnt> chain;
  for (std::size_t i = 0; i < edges.size(); ++i) {
    const std::vector<gp_Pnt> points = edge_points(edges[i], linear, angular);
    // Consecutive edges share their junction vertex: skip the repeat.
    for (std::size_t k = (i == 0 ? 0 : 1); k < points.size(); ++k) {
      chain.push_back(points[k]);
    }
  }
  const bool closed = BRep_Tool::IsClosed(wire);
  if (closed && chain.size() > 1) {
    chain.pop_back(); // the closing vertex is implied
  }
  kinds.push_back(closed ? 1 : 0);
  counts.push_back(static_cast<uint32_t>(chain.size()));
  for (const gp_Pnt &p : chain) {
    push_point(data, p);
  }
}

// --------------------------------------------------------------------------
// Primitives (BRepPrimAPI) in a frame
// --------------------------------------------------------------------------

inline std::unique_ptr<TopoDS_Shape> make_box(rust::Slice<const double> frame, double dx, double dy, double dz) {
  BRepPrimAPI_MakeBox maker(frame_of(frame, "make_box"), dx, dy, dz);
  return boxed(maker.Shape());
}

inline std::unique_ptr<TopoDS_Shape> make_sphere(rust::Slice<const double> frame, double radius) {
  BRepPrimAPI_MakeSphere maker(frame_of(frame, "make_sphere"), radius);
  return boxed(maker.Shape());
}

inline std::unique_ptr<TopoDS_Shape> make_cylinder(rust::Slice<const double> frame, double radius, double height) {
  BRepPrimAPI_MakeCylinder maker(frame_of(frame, "make_cylinder"), radius, height);
  return boxed(maker.Shape());
}

inline std::unique_ptr<TopoDS_Shape> make_cone(rust::Slice<const double> frame, double radius1, double radius2,
                                               double height) {
  BRepPrimAPI_MakeCone maker(frame_of(frame, "make_cone"), radius1, radius2, height);
  return boxed(maker.Shape());
}

// --------------------------------------------------------------------------
// Wires and compounds (the inputs of the sweeps and booleans)
// --------------------------------------------------------------------------

// A polyline wire over flat xyz triples: one BRepBuilderAPI_MakeEdge per
// segment into a BRepBuilderAPI_MakeWire — the fork's cicada_extrude_polygon
// construction, so a prism over a closed polyline wire is byte-identical to
// the fork's prism over the same points. `closed` adds the last → first
// segment.
inline std::unique_ptr<TopoDS_Shape> make_polyline_wire(rust::Slice<const double> xyz, bool closed) {
  if (xyz.size() % 3 != 0) {
    fail("make_polyline_wire: flat xyz buffer length " + std::to_string(xyz.size()) + " is not a multiple of 3");
  }
  const std::size_t count = xyz.size() / 3;
  if (count < (closed ? 3u : 2u)) {
    fail("make_polyline_wire: " + std::to_string(count) + " points is too few");
  }
  BRepBuilderAPI_MakeWire wire_maker;
  const std::size_t segments = closed ? count : count - 1;
  for (std::size_t i = 0; i < segments; ++i) {
    const std::size_t j = (i + 1) % count;
    const gp_Pnt a(xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]);
    const gp_Pnt b(xyz[3 * j], xyz[3 * j + 1], xyz[3 * j + 2]);
    BRepBuilderAPI_MakeEdge edge_maker(a, b);
    wire_maker.Add(edge_maker.Edge());
  }
  require_done(wire_maker, "make_polyline_wire");
  return boxed(wire_maker.Wire());
}

// A full circle as a one-edge wire: center and x axis from the frame,
// normal = the frame's z.
inline std::unique_ptr<TopoDS_Shape> make_circle_wire(rust::Slice<const double> frame, double radius) {
  const gp_Circ circ(frame_of(frame, "make_circle_wire"), radius);
  BRepBuilderAPI_MakeEdge edge_maker(circ);
  BRepBuilderAPI_MakeWire wire_maker(edge_maker.Edge());
  require_done(wire_maker, "make_circle_wire");
  return boxed(wire_maker.Wire());
}

inline std::unique_ptr<TopoDS_Shape> make_compound() {
  BRep_Builder builder;
  TopoDS_Compound compound;
  builder.MakeCompound(compound);
  return boxed(compound);
}

inline void compound_add(TopoDS_Shape &compound, const TopoDS_Shape &shape) {
  if (compound.ShapeType() != TopAbs_COMPOUND) {
    fail("compound_add: the container is not a compound");
  }
  if (shape.IsNull()) {
    fail("compound_add: null shape");
  }
  BRep_Builder builder;
  builder.Add(compound, shape);
}

// --------------------------------------------------------------------------
// Sweeps
// --------------------------------------------------------------------------

// A closed planar wire extruded along (dx, dy, dz): MakeFace(OnlyPlane =
// false) + MakePrism(Copy = false, Canonize = true), the fork's path.
inline std::unique_ptr<TopoDS_Shape> prism(const TopoDS_Shape &profile, double dx, double dy, double dz) {
  const TopoDS_Face face = face_of(profile, "prism");
  BRepPrimAPI_MakePrism maker(face, gp_Vec(dx, dy, dz), /*Copy=*/Standard_False, /*Canonize=*/Standard_True);
  return boxed(maker.Shape());
}

// BRepOffsetAPI_ThruSections over the wires of `sections` (in order), as a
// solid; `ruled` = straight sections (GH Loft "Straight"), otherwise a
// smooth B-spline through them (GH "Normal"). `apex` is empty or one xyz:
// the optional point the last section converges to (extrude_to_point).
// CheckCompatibility stays on: sections get the same edge count, matched
// orientation and aligned seams before the surface is built.
inline std::unique_ptr<TopoDS_Shape> thru_sections(const TopoDS_Shape &sections, bool ruled,
                                                   rust::Slice<const double> apex) {
  if (!(apex.size() == 0 || apex.size() == 3)) {
    fail("thru_sections: apex is empty or one xyz triple, got " + std::to_string(apex.size()) + " doubles");
  }
  BRepOffsetAPI_ThruSections generator(/*isSolid=*/Standard_True, ruled ? Standard_True : Standard_False,
                                       /*pres3d=*/1.0e-6);
  std::int32_t count = 0;
  for (TopoDS_Iterator it(sections); it.More(); it.Next(), ++count) {
    if (it.Value().ShapeType() != TopAbs_WIRE) {
      fail("thru_sections: section " + std::to_string(count) + " is not a wire");
    }
    generator.AddWire(TopoDS::Wire(it.Value()));
  }
  if (apex.size() == 3) {
    BRepBuilderAPI_MakeVertex vertex_maker(gp_Pnt(apex[0], apex[1], apex[2]));
    generator.AddVertex(vertex_maker.Vertex());
  }
  if (count + static_cast<std::int32_t>(apex.size() == 3 ? 1 : 0) < 2) {
    fail("thru_sections: at least two sections are needed");
  }
  generator.CheckCompatibility(Standard_True);
  generator.Build();
  require_done(generator, "BRepOffsetAPI_ThruSections");
  return boxed(generator.Shape());
}

// A closed planar wire revolved about the axis through (ax, ay, az) along
// (dx, dy, dz) by `angle` radians (a full turn when angle reaches 2π).
inline std::unique_ptr<TopoDS_Shape> revolve(const TopoDS_Shape &profile, rust::Slice<const double> axis,
                                             double angle) {
  if (axis.size() != 6) {
    fail("revolve: an axis is 6 doubles (point, direction), got " + std::to_string(axis.size()));
  }
  const TopoDS_Face face = face_of(profile, "revolve");
  const gp_Ax1 ax(gp_Pnt(axis[0], axis[1], axis[2]), gp_Dir(axis[3], axis[4], axis[5]));
  if (std::abs(angle - 2.0 * M_PI) <= Precision::Angular() * 10.0) {
    BRepPrimAPI_MakeRevol maker(face, ax, /*Copy=*/Standard_False);
    return boxed(maker.Shape());
  }
  BRepPrimAPI_MakeRevol maker(face, ax, angle, /*Copy=*/Standard_False);
  return boxed(maker.Shape());
}

// A closed wire swept along a spine wire (BRepOffsetAPI_MakePipeShell, the
// corrected-Frenet trihedron, right-corner = mitred transitions at the
// spine's corners), closed into a solid.
inline std::unique_ptr<TopoDS_Shape> sweep(const TopoDS_Shape &spine, const TopoDS_Shape &profile) {
  if (spine.ShapeType() != TopAbs_WIRE) {
    fail("sweep: the spine must be a wire");
  }
  if (profile.ShapeType() != TopAbs_WIRE) {
    fail("sweep: the profile must be a wire");
  }
  BRepOffsetAPI_MakePipeShell maker(TopoDS::Wire(spine));
  maker.SetMode(/*IsFrenet=*/Standard_False);
  maker.SetTransitionMode(BRepBuilderAPI_RightCorner);
  maker.Add(profile, /*WithContact=*/Standard_False, /*WithCorrection=*/Standard_False);
  maker.Build();
  require_done(maker, "BRepOffsetAPI_MakePipeShell");
  if (!maker.MakeSolid()) {
    fail("BRepOffsetAPI_MakePipeShell could not close the sweep into a solid");
  }
  return boxed(maker.Shape());
}

// --------------------------------------------------------------------------
// Booleans (BRepAlgoAPI, sequential), unified afterwards
// --------------------------------------------------------------------------

// Fuse: the children of `arguments` with the children of `tools` in one
// general-fuse pass (n-ary union). Result as OCCT returns it (a compound);
// the Rust side requires exactly one solid.
inline std::unique_ptr<TopoDS_Shape> fuse(const TopoDS_Shape &arguments, const TopoDS_Shape &tools) {
  BRepAlgoAPI_Fuse op;
  op.SetArguments(children_of(arguments));
  op.SetTools(children_of(tools));
  op.SetRunParallel(Standard_False);
  op.Build();
  require_boolean(op, "BRepAlgoAPI_Fuse");
  return boxed(unified(op.Shape()));
}

// Cut: `shape` minus every child of `tools`, one pass.
inline std::unique_ptr<TopoDS_Shape> cut(const TopoDS_Shape &shape, const TopoDS_Shape &tools) {
  TopTools_ListOfShape arguments;
  arguments.Append(shape);
  BRepAlgoAPI_Cut op;
  op.SetArguments(arguments);
  op.SetTools(children_of(tools));
  op.SetRunParallel(Standard_False);
  op.Build();
  require_boolean(op, "BRepAlgoAPI_Cut");
  return boxed(unified(op.Shape()));
}

inline std::unique_ptr<TopoDS_Shape> common(const TopoDS_Shape &a, const TopoDS_Shape &b) {
  BRepAlgoAPI_Common op(a, b);
  require_boolean(op, "BRepAlgoAPI_Common");
  return boxed(unified(op.Shape()));
}

// --------------------------------------------------------------------------
// Measurement, bounds, transform
// --------------------------------------------------------------------------

// Volume and centroid (BRepGProp::VolumeProperties, adaptive to `eps`);
// `out` receives [volume, cx, cy, cz].
inline void volume_properties(const TopoDS_Shape &shape, double eps, rust::Vec<double> &out) {
  GProp_GProps props;
  BRepGProp::VolumeProperties(shape, props, eps, /*OnlyClosed=*/Standard_True, /*SkipShared=*/Standard_False);
  const gp_Pnt c = props.CentreOfMass();
  out.clear();
  out.push_back(props.Mass());
  out.push_back(c.X());
  out.push_back(c.Y());
  out.push_back(c.Z());
}

// The tight world-aligned bounds (BRepBndLib::AddOptimal on the surfaces,
// no triangulation, no tolerance inflation); `out` receives
// [xmin, ymin, zmin, xmax, ymax, zmax].
inline void bounds(const TopoDS_Shape &shape, rust::Vec<double> &out) {
  Bnd_Box box;
  BRepBndLib::AddOptimal(shape, box, /*useTriangulation=*/Standard_False, /*useShapeTolerance=*/Standard_False);
  if (box.IsVoid()) {
    fail("bounds: the shape has no extent");
  }
  Standard_Real xmin = 0, ymin = 0, zmin = 0, xmax = 0, ymax = 0, zmax = 0;
  box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
  out.clear();
  out.push_back(xmin);
  out.push_back(ymin);
  out.push_back(zmin);
  out.push_back(xmax);
  out.push_back(ymax);
  out.push_back(zmax);
}

// BRepBuilderAPI_Transform with Copy = true (BRepTools_Modifier under it)
// rebuilds every edge with the transformed 3D curve and pcurves on the
// transformed surfaces — and, measured 2026-08-21 on a sphere's degenerate
// pole edges, KEEPS the source edge's pcurve on the SOURCE surface beside
// them: the moved sphere serialized with two spherical surfaces, the second
// one at the original centre, referenced by no face. The stale
// representation rides into every later boolean (the bytes grow; the result
// is still BRepCheck-valid) and the mesher, meeting it where an intersection
// curve runs through the pole, discretizes that edge differently for the two
// faces: 159 T-junctions, a mesh that does not close (the moved sphere minus
// a cylinder through both poles; its twin built in place meshed closed).
// Drop every pcurve whose surface is not the surface of some face of the
// shape, so a moved solid carries exactly its moved geometry — after which
// the moved sphere serializes to the same size as its twin and the cut
// meshes closed with the twin's triangle count (occt/node_set_tests.rs).
inline void drop_foreign_pcurves(const TopoDS_Shape &shape) {
  std::vector<Handle(Geom_Surface)> own;
  for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
    TopLoc_Location location;
    own.push_back(BRep_Tool::Surface(TopoDS::Face(it.Current()), location));
  }
  const auto is_own = [&own](const Handle(Geom_Surface) &surface) {
    for (const Handle(Geom_Surface) &candidate : own) {
      if (candidate == surface) {
        return true;
      }
    }
    return false;
  };
  ShapeBuild_Edge edge_tool;
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(shape, TopAbs_EDGE, edges);
  for (Standard_Integer i = 1; i <= edges.Extent(); ++i) {
    const TopoDS_Edge &edge = TopoDS::Edge(edges(i));
    const Handle(BRep_TEdge) tedge = Handle(BRep_TEdge)::DownCast(edge.TShape());
    if (tedge.IsNull()) {
      continue;
    }
    std::vector<Handle(Geom_Surface)> foreign;
    for (BRep_ListIteratorOfListOfCurveRepresentation rep(tedge->Curves()); rep.More(); rep.Next()) {
      if (rep.Value()->IsCurveOnSurface() && !is_own(rep.Value()->Surface())) {
        foreign.push_back(rep.Value()->Surface());
      }
    }
    for (const Handle(Geom_Surface) &surface : foreign) {
      edge_tool.RemovePCurve(edge, surface);
    }
  }
}

// A similarity (rotation × uniform scale, reflections included, plus a
// translation) as the 12 row-major coefficients of its 3×4 matrix, applied
// with the geometry COPIED and rewritten (BRepBuilderAPI_Transform with
// Copy = true): the result carries no TopLoc_Location, so its canonical
// bytes describe the moved geometry itself. gp_Trsf::SetValues refuses a
// singular matrix and re-orthogonalizes the rest.
inline std::unique_ptr<TopoDS_Shape> transform(const TopoDS_Shape &shape, rust::Slice<const double> m) {
  if (m.size() != 12) {
    fail("transform: 12 row-major coefficients expected, got " + std::to_string(m.size()));
  }
  gp_Trsf trsf;
  trsf.SetValues(m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11]);
  BRepBuilderAPI_Transform maker(shape, trsf, /*Copy=*/Standard_True);
  require_done(maker, "BRepBuilderAPI_Transform");
  TopoDS_Shape moved = maker.Shape();
  drop_foreign_pcurves(moved);
  return boxed(moved);
}

// --------------------------------------------------------------------------
// Topology readers
// --------------------------------------------------------------------------

// BRepCheck_Analyzer's verdict on the whole shape (geometry and topology
// checked): the kernel's own notion of a valid B-rep. Diagnostic — a
// boolean can return a solid the analyzer accepts whose mesh still does not
// close (the unclosed-tessellation regression in occt/tests.rs).
inline bool is_valid(const TopoDS_Shape &shape) {
  if (shape.IsNull()) {
    fail("is_valid: null shape");
  }
  BRepCheck_Analyzer analyzer(shape, /*GeomControls=*/Standard_True);
  return analyzer.IsValid() != 0;
}

// The i-th TopAbs_SOLID sub-shape (0-based, explorer order).
inline std::unique_ptr<TopoDS_Shape> nth_solid(const TopoDS_Shape &shape, std::int32_t index) {
  std::int32_t seen = 0;
  for (TopExp_Explorer it(shape, TopAbs_SOLID); it.More(); it.Next(), ++seen) {
    if (seen == index) {
      return boxed(it.Current());
    }
  }
  fail("nth_solid: index " + std::to_string(index) + " out of range (" + std::to_string(seen) + " solids)");
}

// Every distinct edge (degenerate ones skipped) as curve records, and the
// face count, for deconstruct_solid.
inline std::int32_t edges(const TopoDS_Shape &shape, double linear, double angular, rust::Vec<int32_t> &kinds,
                          rust::Vec<uint32_t> &counts, rust::Vec<double> &data) {
  kinds.clear();
  counts.clear();
  data.clear();
  TopTools_IndexedMapOfShape map;
  TopExp::MapShapes(shape, TopAbs_EDGE, map);
  for (Standard_Integer i = 1; i <= map.Extent(); ++i) {
    const TopoDS_Edge &edge = TopoDS::Edge(map(i));
    if (BRep_Tool::Degenerated(edge)) {
      continue;
    }
    push_edge(kinds, counts, data, edge, linear, angular);
  }
  std::int32_t faces = 0;
  for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
    ++faces;
  }
  return faces;
}

// Every distinct vertex as xyz triples.
inline void vertices(const TopoDS_Shape &shape, rust::Vec<double> &out) {
  out.clear();
  TopTools_IndexedMapOfShape map;
  TopExp::MapShapes(shape, TopAbs_VERTEX, map);
  for (Standard_Integer i = 1; i <= map.Extent(); ++i) {
    push_point(out, BRep_Tool::Pnt(TopoDS::Vertex(map(i))));
  }
}

// The planar section of a solid: BRepAlgoAPI_Section against the plane
// through `plane[0..3]` with normal `plane[3..6]`, its edges connected into
// wires at `tolerance`, each wire one curve record (circles exact, the rest
// discretized at the deflections).
inline void section(const TopoDS_Shape &shape, rust::Slice<const double> plane, double tolerance, double linear,
                    double angular, rust::Vec<int32_t> &kinds, rust::Vec<uint32_t> &counts,
                    rust::Vec<double> &data) {
  if (plane.size() != 6) {
    fail("section: a plane is 6 doubles (origin, normal), got " + std::to_string(plane.size()));
  }
  kinds.clear();
  counts.clear();
  data.clear();
  const gp_Pln pln(gp_Pnt(plane[0], plane[1], plane[2]), gp_Dir(plane[3], plane[4], plane[5]));
  BRepAlgoAPI_Section op(shape, pln, /*PerformNow=*/Standard_False);
  op.ComputePCurveOn1(Standard_False);
  op.Approximation(Standard_False);
  op.SetRunParallel(Standard_False);
  op.Build();
  require_boolean(op, "BRepAlgoAPI_Section");
  Handle(TopTools_HSequenceOfShape) edges = new TopTools_HSequenceOfShape();
  for (TopExp_Explorer it(op.Shape(), TopAbs_EDGE); it.More(); it.Next()) {
    edges->Append(it.Current());
  }
  if (edges->IsEmpty()) {
    return; // the plane misses the solid: no curves
  }
  Handle(TopTools_HSequenceOfShape) wires;
  ShapeAnalysis_FreeBounds::ConnectEdgesToWires(edges, tolerance, /*shared=*/Standard_False, wires);
  if (wires.IsNull()) {
    fail("section: connecting the section edges into wires failed");
  }
  for (Standard_Integer i = 1; i <= wires->Length(); ++i) {
    push_wire(kinds, counts, data, TopoDS::Wire(wires->Value(i)), linear, angular);
  }
}

// --------------------------------------------------------------------------
// STEP (global state: the caller holds the STEP lock)
// --------------------------------------------------------------------------

// Lower OCCT's default printers to failures only. The STEP translators
// narrate every transfer at Info level on stdout ("*** Write Done ***", the
// statistics tables); a headless `cicada run` must print its own output and
// nothing else. Errors still surface — as Err, through the status checks.
inline void quiet_messenger() {
  const Handle(Message_Messenger) &messenger = Message::DefaultMessenger();
  for (Message_SequenceOfPrinters::Iterator it(messenger->Printers()); it.More(); it.Next()) {
    it.Value()->SetTraceLevel(Message_Fail);
  }
}

inline UnitsMethods_LengthUnit length_unit_of(double millimeters) {
  const UnitsMethods_LengthUnit unit =
      UnitsMethods::GetLengthUnitByFactorValue(millimeters, UnitsMethods_LengthUnit_Millimeter);
  if (unit == UnitsMethods_LengthUnit_Undefined) {
    fail("STEP: no STEP length unit corresponds to " + std::to_string(millimeters) + " mm");
  }
  return unit;
}

// Write the children of `shapes` to a STEP AP214 file, the document's unit
// declared in the file (`millimeters` = mm per document unit), with a header
// whose every field is fixed — name, the timestamp `timestamp`, author,
// organisation, authorisation — so the same solids give the same bytes.
inline void step_write(const TopoDS_Shape &shapes, rust::Str path, double millimeters, rust::Str name,
                       rust::Str timestamp) {
  const std::string path_s(path);
  const std::string name_s(name);
  const std::string timestamp_s(timestamp);
  StepData_ConfParameters params;
  params.WriteSchema = StepData_ConfParameters::WriteMode_StepSchema_AP214IS;
  params.WriteUnit = length_unit_of(millimeters);
  params.WriteProductName = name_s.c_str();
  STEPControl_Writer writer;
  Handle(StepData_StepModel) model = writer.Model(Standard_True);
  model->SetLocalLengthUnit(millimeters);
  model->SetWriteLengthUnit(millimeters);
  std::int32_t count = 0;
  for (TopoDS_Iterator it(shapes); it.More(); it.Next(), ++count) {
    if (writer.Transfer(it.Value(), STEPControl_AsIs, params) != IFSelect_RetDone) {
      fail("STEP: transferring solid " + std::to_string(count) + " failed");
    }
  }
  if (count == 0) {
    fail("STEP: nothing to write");
  }
  APIHeaderSection_MakeHeader header(model);
  if (!header.HasFn()) {
    APIHeaderSection_MakeHeader fresh;
    fresh.Apply(model);
    header = APIHeaderSection_MakeHeader(model);
  }
  if (!header.HasFn()) {
    fail("STEP: the model has no FILE_NAME header to normalize");
  }
  // The writer names each PRODUCT "<name> <n>" with n from a PROCESS-WIDE
  // counter (measured: a second export in the same process wrote 'parts 11'
  // where the first wrote 'parts 1'). Renumber from 1 in this file's order
  // so the same solids always give the same bytes.
  std::int32_t product = 0;
  for (Standard_Integer i = 1; i <= model->NbEntities(); ++i) {
    Handle(StepBasic_Product) entity = Handle(StepBasic_Product)::DownCast(model->Value(i));
    if (entity.IsNull()) {
      continue;
    }
    const std::string label = name_s + " " + std::to_string(++product);
    entity->SetId(new TCollection_HAsciiString(label.c_str()));
    entity->SetName(new TCollection_HAsciiString(label.c_str()));
  }
  header.SetName(new TCollection_HAsciiString(name_s.c_str()));
  header.SetTimeStamp(new TCollection_HAsciiString(timestamp_s.c_str()));
  header.SetAuthorValue(1, new TCollection_HAsciiString("cicada"));
  header.SetOrganizationValue(1, new TCollection_HAsciiString("cicada"));
  header.SetOriginatingSystem(new TCollection_HAsciiString("cicada"));
  header.SetAuthorisation(new TCollection_HAsciiString("cicada"));
  if (writer.Write(path_s.c_str()) != IFSelect_RetDone) {
    fail("STEP: writing `" + path_s + "` failed");
  }
}

// Read a STEP file into one shape (the roots as a compound when there are
// several), scaled into the document's unit (`millimeters` = mm per
// document unit). The Rust side counts and extracts the solids.
inline std::unique_ptr<TopoDS_Shape> step_read(rust::Str path, double millimeters) {
  const std::string path_s(path);
  StepData_ConfParameters params;
  STEPControl_Reader reader;
  const IFSelect_ReturnStatus status = reader.ReadFile(path_s.c_str(), params);
  if (status != IFSelect_RetDone) {
    fail("STEP: reading `" + path_s + "` failed (status " + std::to_string(static_cast<int>(status)) + ")");
  }
  reader.SetSystemLengthUnit(millimeters);
  const Standard_Integer roots = reader.TransferRoots();
  if (roots == 0) {
    fail("STEP: `" + path_s + "` holds no transferable shape");
  }
  const TopoDS_Shape shape = reader.OneShape();
  if (shape.IsNull()) {
    fail("STEP: `" + path_s + "` produced a null shape");
  }
  return boxed(shape);
}

} // namespace cicada_geom
