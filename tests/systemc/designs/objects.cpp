// C++ classes in process bodies: constructors with initializer lists,
// inheritance and virtual functions, pointers to objects (shared) versus
// objects by value (copied on assignment and when passed by value),
// static members, new/delete.
#include <systemc.h>
#include <string>
class shape {
public:
    shape(const std::string& n) : name(n) { count++; }
    virtual ~shape() {}
    virtual int area() const { return 0; }
    virtual std::string kind() const { return "shape"; }
    std::string describe() const { return kind() + " " + name; }
    static int count;
protected:
    std::string name;
};
int shape::count = 0;
class rect : public shape {
public:
    rect(const std::string& n, int w, int h) : shape(n), w(w), h(h) {}
    int area() const override { return w * h; }
    std::string kind() const override { return "rect"; }
    void scale(int k) { w *= k; h *= k; }
    int w, h;
};
class square : public rect {
public:
    square(const std::string& n, int s) : rect(n, s, s) {}
    std::string kind() const override { return "square"; }
};
struct point { int x, y; point() : x(0), y(0) {} point(int a, int b) : x(a), y(b) {} };
int manhattan(point p) { p.x = p.x < 0 ? -p.x : p.x; return p.x + (p.y < 0 ? -p.y : p.y); }
void move(point& p, int dx) { p.x += dx; }
SC_MODULE(m) {
    void run() {
        rect r("r1", 2, 3);
        square sq("s1", 4);
        shape* shapes[3];
        shapes[0] = &r;
        shapes[1] = &sq;
        shapes[2] = new rect("r2", 5, 1);
        for (int i = 0; i < 3; i++)
            std::cout << shapes[i]->describe() << " area " << shapes[i]->area() << std::endl;
        rect copy = r;      // a copy: scaling it leaves r alone
        copy.scale(10);
        rect* alias = &r;   // a pointer: scaling through it changes r
        alias->scale(2);
        std::cout << "r " << r.area() << " copy " << copy.area() << " count " << shape::count << std::endl;
        point a(-3, 4);
        std::cout << "manhattan " << manhattan(a) << " a.x " << a.x << std::endl;
        move(a, 10);
        point b = a;
        b.y = 0;
        std::cout << "a " << a.x << "," << a.y << " b " << b.x << "," << b.y << std::endl;
        delete shapes[2];
        wait(1, SC_NS);
        std::cout << sc_time_stamp() << " done" << std::endl;
    }
    SC_CTOR(m) { SC_THREAD(run); }
};
int sc_main(int, char*[]) {
    m u("u");
    sc_start();
    return 0;
}
