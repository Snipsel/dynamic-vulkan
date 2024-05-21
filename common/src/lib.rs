// TODO: think about colors. Right now Color means sRGBA8
// maybe have f32 oklab colors internally and use .srgba8() to convert?
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Color{
    r:u8, g:u8, b:u8, a:u8
}
impl Color{
    pub const CLEAR:Color = Color{r:0x00, g:0x00, b:0x00, a:0x00};
    pub const WHITE:Color = Color{r:0xFF, g:0xFF, b:0xFF, a:0xFF};
    pub const BLACK:Color = Color{r:0x00, g:0x00, b:0x00, a:0xFF};
    pub fn srgb8(r:u8, g:u8, b:u8, a:u8) -> Self{ Self{r,g,b,a} }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Vertex{
    pub x:i16,
    pub y:i16,
    pub u:u16,
    pub v:u16,
    pub color: Color
}

pub fn gen_quad(x: i16, y: i16, w: i16, h: i16, u:u16, v:u16, color: Color) -> [Vertex;4] {
    assert!(w > 0);
    assert!(h > 0);
    let w_ = w as u16;
    let h_ = h as u16;
    [
        Vertex{x:x+0,  y:y+0, u:u+0,  v:v+0,  color}, // top left
        Vertex{x:x+0,  y:y+h, u:u+0,  v:v+h_, color}, // bottom left
        Vertex{x:x+w,  y:y+0, u:u+w_, v:v+0,  color}, // top right
        Vertex{x:x+w,  y:y+h, u:u+w_, v:v+h_, color}, // bottom right
    ]
}

pub fn gen_rect(x:i16, y:i16, w:i16, h:i16, color: Color) -> [Vertex;4] {
    assert!(w > 0);
    assert!(h > 0);
    [
        Vertex{x:x+0,  y:y+0, u:0xFFFF, v:0xFFFF, color}, // top left
        Vertex{x:x+0,  y:y+h, u:0xFFFF, v:0xFFFF, color}, // bottom left
        Vertex{x:x+w,  y:y+0, u:0xFFFF, v:0xFFFF, color}, // top right
        Vertex{x:x+w,  y:y+h, u:0xFFFF, v:0xFFFF, color}, // bottom right
    ]
}

pub fn div_round(a:i32, b:i32) -> i32 { (a+(b/2))/b }

use core::ops::*;

// pub const fn q6(x:i32) -> i32q6 { i32q6(x<<6) }

// #[repr(transparent)]
// #[derive(Clone,Copy)]
// pub struct i32q6(pub(super)i32);
// impl From<i32> for i32q6 { fn from(value: i32) -> Self { Self(value<<6) } }
// impl Add for i32q6 { type Output=Self; fn add(self, rhs: Self) -> Self { Self(self.0+rhs.0) } }
// impl Sub for i32q6 { type Output=Self; fn sub(self, rhs: Self) -> Self { Self(self.0-rhs.0) } }
// impl Mul for i32q6 { type Output=Self; fn mul(self, rhs: Self) -> Self { Self((self.0*rhs.0)>>6) } }
// impl AddAssign      for i32q6 { fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0;  } }
// impl SubAssign      for i32q6 { fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0;  } }
// impl AddAssign<i32> for i32q6 { fn add_assign(&mut self, rhs: i32)  { self.0 += rhs<<6; } }
// impl SubAssign<i32> for i32q6 { fn sub_assign(&mut self, rhs: i32)  { self.0 -= rhs<<6; } }

#[allow(non_camel_case_types)]
#[derive(Clone,Copy)]
pub struct vec2<T>(pub T,pub T) where T:Clone+Copy;
impl<T> Add for vec2<T> where T:Clone+Copy+Add<Output=T> { type Output=Self; fn add(self, rhs: Self) -> Self { Self(self.0+rhs.0, self.1+rhs.1) } }
impl<T> Sub for vec2<T> where T:Clone+Copy+Sub<Output=T> { type Output=Self; fn sub(self, rhs: Self) -> Self { Self(self.0-rhs.0, self.1-rhs.1) } }
impl<T> Mul<T> for vec2<T> where T:Clone+Copy+Mul<Output=T> { type Output=Self; fn mul(self, rhs: T) -> Self { Self(self.0*rhs, self.1*rhs) } }
impl<T> Div<T> for vec2<T> where T:Clone+Copy+Div<Output=T> { type Output=Self; fn div(self, rhs: T) -> Self { Self(self.0/rhs, self.1/rhs) } }
impl<T,U> AddAssign<vec2<U>> for vec2<T> where T:Clone+Copy+AddAssign<U>, U:Clone+Copy { fn add_assign(&mut self, rhs: vec2<U>) { self.0 += rhs.0; self.1 += rhs.1; } }
impl<T,U> SubAssign<vec2<U>> for vec2<T> where T:Clone+Copy+SubAssign<U>, U:Clone+Copy { fn sub_assign(&mut self, rhs: vec2<U>) { self.0 -= rhs.0; self.1 -= rhs.1; } }

